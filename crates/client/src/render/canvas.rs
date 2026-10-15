//! Painted skins' canvases: the items a skin can be a picture on, and the
//! meshes whose UVs that picture is laid on.
//!
//! A painted skin is one square PNG (`content/skins.toml`'s `texture`) drawn
//! as the item's whole surface colour. The UV layout it is painted against
//! is therefore **a public contract** (`DECISIONS.md:432`): once a skin
//! exists for an item, that item's UVs can never move, or every skin painted
//! for it lands in the wrong place. `tests/skin_templates.rs` hashes each
//! canvas's UVs and triangles, and `skinkit template` draws the same layout
//! for a painter, from these same meshes.
//!
//! Two canvases so far, both shapes this crate builds in code: the bat (one
//! lathe, wrapped once around, tip at the top of the image) and the Burlap
//! Tunic (its top in the upper half, its skirt in the lower). The modelled
//! tools' Meshy unwraps are hundreds of islands each and wait for an
//! authored unwrap (phase 2).

use bevy::mesh::{Indices, Mesh, VertexAttributeValues};

/// A named band of the image, for a painter: `v` from the top edge (0) to
/// the bottom (1).
#[derive(Clone, Copy, Debug)]
pub struct Region {
    pub name: &'static str,
    pub v: (f32, f32),
}

/// An item a painted skin can cover.
#[derive(Clone, Copy, Debug)]
pub struct Canvas {
    /// The content id (`content/items.toml`), as `content/skins.toml`'s
    /// `covers` names it.
    pub item: &'static str,
    /// What `skinkit` calls it.
    pub key: &'static str,
    /// Where the picture goes, top to bottom.
    pub regions: &'static [Region],
    /// What the picture wraps: said once to a painter.
    pub wrap: &'static str,
}

/// Every canvas. `content::validate`'s `SKIN_CANVAS_ITEMS` must name the
/// same items (`tests/skin_templates.rs`).
pub const CANVASES: [Canvas; 2] = [
    Canvas {
        item: "item.bat",
        key: "bat",
        regions: &[
            // Measured off the profile's arc length (`heldgen::bat_mesh`):
            // the dome from y 0.79, the barrel down to the taper's middle
            // (y 0.45), the handle down to the knob (y 0.034).
            Region {
                name: "dome (tip)",
                v: (0.0, 0.085),
            },
            Region {
                name: "barrel",
                v: (0.085, 0.476),
            },
            Region {
                name: "handle",
                v: (0.476, 0.938),
            },
            Region {
                name: "knob",
                v: (0.938, 1.0),
            },
        ],
        wrap: "once around: left edge to right edge is one turn, the seam faces +X",
    },
    Canvas {
        item: "item.armor_burlap_body",
        key: "tunic",
        regions: &[
            Region {
                name: "top: collar to waist",
                v: (0.0, 0.5),
            },
            Region {
                name: "skirt: waist to hem",
                v: (0.5, 1.0),
            },
        ],
        wrap: "once around: left edge to right edge is one turn starting at the chest, running toward the left arm",
    },
];

/// The canvas a content id or `skinkit` key names.
pub fn find(name: &str) -> Option<&'static Canvas> {
    CANVASES.iter().find(|c| c.item == name || c.key == name)
}

/// The meshes a canvas's picture is laid on, as the game draws them.
pub fn meshes(canvas: &Canvas) -> Vec<Mesh> {
    match canvas.key {
        "bat" => vec![super::heldgen::canvas_mesh("bat").expect("the bat is a canvas")],
        "tunic" => super::worn::tunic_meshes().to_vec(),
        other => panic!("canvas {other:?} has no meshes"),
    }
}

/// One mesh's positions, normals, UVs and triangles, read back out of the
/// Bevy mesh for code that is not a renderer (`skinkit`, the tests).
pub struct Flat {
    pub pos: Vec<[f32; 3]>,
    pub nrm: Vec<[f32; 3]>,
    pub uv: Vec<[f32; 2]>,
    pub tris: Vec<[u32; 3]>,
}

pub fn flat(mesh: &Mesh) -> Flat {
    let pos = match mesh.attribute(Mesh::ATTRIBUTE_POSITION) {
        Some(VertexAttributeValues::Float32x3(v)) => v.clone(),
        _ => Vec::new(),
    };
    let nrm = match mesh.attribute(Mesh::ATTRIBUTE_NORMAL) {
        Some(VertexAttributeValues::Float32x3(v)) => v.clone(),
        _ => vec![[0.0, 1.0, 0.0]; pos.len()],
    };
    let uv = match mesh.attribute(Mesh::ATTRIBUTE_UV_0) {
        Some(VertexAttributeValues::Float32x2(v)) => v.clone(),
        _ => Vec::new(),
    };
    let idx: Vec<u32> = match mesh.indices() {
        Some(Indices::U32(i)) => i.clone(),
        Some(Indices::U16(i)) => i.iter().map(|&x| u32::from(x)).collect(),
        None => (0..pos.len() as u32).collect(),
    };
    let tris = idx.chunks_exact(3).map(|t| [t[0], t[1], t[2]]).collect();
    Flat { pos, nrm, uv, tris }
}
