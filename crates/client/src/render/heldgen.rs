//! Generated held-item geometry: the meshes behind `ui::hold::HeldSrc::Gen`
//! rows, and the two-primitive stand-in tool the viewmodel falls back to.
//!
//! **Why generated rather than sourced.** The revolver is 0.5.0's headline
//! weapon and the torch is in every starter kit, and neither has a `.glb` —
//! `assets/models/WANTED.md` carries both. Until one lands, holding either
//! drew the generic stand-in, which is the exact lie `ui::hold::held_model`'s
//! doc warns about one state earlier: the hand told the player nothing about
//! what swapping items had put in it. A dozen tinted boxes are not art, but
//! they are the RIGHT dozen boxes — a revolver reads as a revolver at 0.5 m.
//! When a real asset lands, the row in `HELD_MODELS` flips from `Gen` to
//! `Glb` and this module loses a match arm; nothing else moves.
//!
//! **The authoring convention is the glb one**, deliberately: +Y up, foot at
//! y = 0, so `HeldModelDef::grip_m` and the quarter-turn in
//! `viewmodel::swap` treat both sources identically, and
//! `tests/held_assets.rs` holds a generated row's measured HEIGHT to its
//! declared `height_m` exactly as it holds a file's.
//!
//! **Nothing here glows, and that is now a narrower claim than it was.**
//! `tests/held_assets.rs::nothing_held_glows` forbids a carried EMISSIVE —
//! a material that is bright in its own right — and it still holds over
//! every row including the torch. What it never forbade, and what this
//! module's own header claimed it did until torch light v0, is a carried
//! **light**: since that slice the torch's row declares one
//! (`ui::hold::TORCH_LIGHT`) and `render::viewmodel::hand_light` hangs a
//! `PointLight` off the hand for it. The two are different mechanisms and
//! the mis-citation here was the exact class `CLAUDE.md` opens with —
//! prose reading as covered while nothing checked it.
//!
//! So the torch has no flame GEOMETRY and no emissive material: it is lit
//! from just above its own crown (`ui::hold::FLAME_LIFT_M`) so its head
//! reads bright without emitting, and a real flame primitive is a VFX
//! slice nobody has looked at yet (`NOW.md` §0tl,
//! `assets/models/MANIFEST.md`).

use bevy::prelude::*;

use super::props::{tint1, Soup};

/// Emit an axis-aligned box into a soup. `c` is the centre, `h` the
/// half-extent, `tint` a mean-1 colour over whatever photograph the material
/// carries (`props::tint1`).
pub(super) fn boxed(s: &mut Soup, c: Vec3, h: Vec3, tint: [f32; 3]) {
    let v = |sx: f32, sy: f32, sz: f32| c + Vec3::new(h.x * sx, h.y * sy, h.z * sz);
    // Per-face value break-up, small: the photograph carries the variation and
    // a second source of it is the patchwork `blob_mesh` used to be.
    let faces = [
        (
            [
                v(-1., -1., 1.),
                v(1., -1., 1.),
                v(1., 1., 1.),
                v(-1., 1., 1.),
            ],
            1.00,
        ),
        (
            [
                v(1., -1., -1.),
                v(-1., -1., -1.),
                v(-1., 1., -1.),
                v(1., 1., -1.),
            ],
            0.93,
        ),
        (
            [
                v(1., -1., 1.),
                v(1., -1., -1.),
                v(1., 1., -1.),
                v(1., 1., 1.),
            ],
            0.97,
        ),
        (
            [
                v(-1., -1., -1.),
                v(-1., -1., 1.),
                v(-1., 1., 1.),
                v(-1., 1., -1.),
            ],
            0.95,
        ),
        (
            [
                v(-1., 1., 1.),
                v(1., 1., 1.),
                v(1., 1., -1.),
                v(-1., 1., -1.),
            ],
            1.02,
        ),
        (
            [
                v(-1., -1., -1.),
                v(1., -1., -1.),
                v(1., -1., 1.),
                v(-1., -1., 1.),
            ],
            0.90,
        ),
    ];
    for (q, shade) in faces {
        let col = move |_: Vec3| [tint[0] * shade, tint[1] * shade, tint[2] * shade, 1.0];
        s.tri(q[0], q[1], q[2], col, None, 0.0);
        s.tri(q[0], q[2], q[3], col, None, 0.0);
    }
}

/// Emit a hexahedron from eight corners: `back` then `front`, each wound
/// counter-clockwise seen from outside its own face. A box cannot taper and a
/// blade must, so this is the shape the axe bit needs and `boxed` cannot make.
pub(super) fn hexa(s: &mut Soup, back: [Vec3; 4], front: [Vec3; 4], tint: [f32; 3]) {
    let quad = |s: &mut Soup, q: [Vec3; 4], shade: f32| {
        let col = move |_: Vec3| [tint[0] * shade, tint[1] * shade, tint[2] * shade, 1.0];
        s.tri(q[0], q[1], q[2], col, None, 0.0);
        s.tri(q[0], q[2], q[3], col, None, 0.0);
    };
    quad(s, front, 1.00);
    quad(s, [back[3], back[2], back[1], back[0]], 0.90);
    quad(s, [back[0], back[1], front[1], front[0]], 0.97);
    quad(s, [back[1], back[2], front[2], front[1]], 1.03);
    quad(s, [back[2], back[3], front[3], front[2]], 0.95);
    quad(s, [back[3], back[0], front[0], front[3]], 0.99);
}

/// The stand-in's handle: a long box down the view's forward axis.
///
/// The stand-in covers every item with no model of its own — since the table
/// grew its deployables and generated rows that is the metal tools, the
/// resources and the small consumables — so it stays deliberately generic:
/// a hafted something, not a claim about the item.
pub(super) fn handle_mesh() -> Mesh {
    // Tiles per metre, and deliberately dense: a haft is ~0.5 m and the plank
    // map is authored for a wall, so at 1.0 the handle shows one blurred
    // plank rather than grain.
    let mut s = Soup::tiling(3.0);
    let grain = tint1(0x8a6f4a);
    boxed(
        &mut s,
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(0.016, 0.019, 0.25),
        grain,
    );
    s.mesh()
}

/// The stand-in's head: an eye around the haft and a bit swept out to one
/// side.
///
/// **The tiling here is 1.1 and the first cut was 9.0, which is the boulder's
/// mistake in miniature.** `CorrugatedSteel009` is photoscanned RIBBED SHEET;
/// at nine tiles per metre a 10 cm head samples a dozen full rib periods and
/// resolves into a stack of pale stripes — it read as a paperback on a stick.
/// At 1.1 the head samples roughly a tenth of the map, so what reaches the
/// frame is the broad tonal variation of worn steel rather than its
/// corrugation. The rule the two share: **match the tile rate to the scale
/// the map was photographed at, not to the size of the object.** A map whose
/// own features are wrong for the identity cannot be fixed by either — that
/// is a sourcing gap, and a plain steel albedo is the entry `MANIFEST.md` is
/// missing.
pub(super) fn head_mesh() -> Mesh {
    // Tiling is irrelevant to this mesh's own material, which carries no map —
    // see `viewmodel::spawn_item`. Kept sane so the UVs are well-formed for
    // tangents.
    let mut s = Soup::tiling(1.0);
    let steel = tint1(0x9aa0a6);
    // The eye, wrapped around the haft.
    boxed(
        &mut s,
        Vec3::new(-0.004, 0.014, -0.238),
        Vec3::new(0.024, 0.040, 0.030),
        steel,
    );
    // The bit, TAPERED — thick and short where it meets the eye, thin and
    // flared at the cutting edge. Two earlier cuts made this out of boxes
    // (one, then three) and both read as a cluster of blocks rather than a
    // blade: at 10 cm the silhouette is the whole of what the eye gets, and a
    // rectangle is not an axe. The flare in Y and the taper in Z are what
    // make the outline read.
    let (zb, zf) = (0.022f32, 0.004f32);
    let (yb, yf) = (0.040f32, 0.060f32);
    let (xb, xf) = (-0.020f32, -0.098f32);
    let y0 = 0.018f32;
    hexa(
        &mut s,
        [
            Vec3::new(xb, y0 - yb, zb),
            Vec3::new(xb, y0 + yb, zb),
            Vec3::new(xb, y0 + yb, -zb),
            Vec3::new(xb, y0 - yb, -zb),
        ],
        [
            Vec3::new(xf, y0 - yf, zf),
            Vec3::new(xf, y0 + yf, zf),
            Vec3::new(xf, y0 + yf, -zf),
            Vec3::new(xf, y0 - yf, -zf),
        ],
        steel,
    );
    s.mesh()
}

/// Revolve a profile around +Y. Radial zero closes a cap; a return along
/// the inner radius leaves an actual bore instead of a painted muzzle.
fn turned(s: &mut Soup, center: Vec3, profile: &[(f32, f32)], tint: [f32; 3]) {
    for pair in profile.windows(2) {
        let [(y0, r0), (y1, r1)] = [pair[0], pair[1]];
        for side in 0..12 {
            let angle = |i: usize| i as f32 * std::f32::consts::TAU / 12.0;
            let point = |y: f32, r: f32, i: usize| {
                center + Vec3::new(angle(i).cos() * r, y, angle(i).sin() * r)
            };
            let a = point(y0, r0, side);
            let b = point(y1, r1, side);
            let c = point(y1, r1, side + 1);
            let d = point(y0, r0, side + 1);
            let color = |_| [tint[0], tint[1], tint[2], 1.0];
            if r1 > 0.0 {
                s.tri(a, b, c, color, None, 0.0);
            }
            if r0 > 0.0 {
                s.tri(a, c, d, color, None, 0.0);
            }
        }
    }
}

/// A tapered wooden shaft and layered cloth winding. The existing crown,
/// grip and light socket stay in place; the silhouette now has round edges.
fn torch_mesh() -> Mesh {
    let mut s = Soup::tiling(3.0);
    turned(
        &mut s,
        Vec3::ZERO,
        &[
            (0.0, 0.0),
            (0.0, 0.010),
            (0.08, 0.013),
            (0.30, 0.011),
            (0.38, 0.013),
            (0.38, 0.0),
        ],
        [0.30, 0.23, 0.15],
    );
    turned(
        &mut s,
        Vec3::ZERO,
        &[
            (0.356, 0.0),
            (0.356, 0.018),
            (0.375, 0.023),
            (0.410, 0.028),
            (0.440, 0.025),
            (0.455, 0.020),
            (0.455, 0.0),
        ],
        [0.16, 0.14, 0.12],
    );
    // The wrap's overlapping edges catch light independently of the core.
    for (y, r) in [
        (0.370, 0.023),
        (0.390, 0.027),
        (0.412, 0.029),
        (0.434, 0.027),
    ] {
        turned(
            &mut s,
            Vec3::ZERO,
            &[
                (y, r - 0.003),
                (y + 0.003, r),
                (y + 0.012, r - 0.001),
                (y + 0.013, r - 0.004),
            ],
            [0.24, 0.21, 0.17],
        );
    }
    s.mesh()
}

/// Barrel-up authoring, as before: +Y becomes forward in the hand. A round
/// cylinder, recessed bore, front sight and open trigger guard carry its read.
fn revolver_mesh() -> Mesh {
    let mut s = Soup::tiling(1.0);
    let steel = [0.29, 0.30, 0.33];
    let grain = [0.24, 0.16, 0.10];
    // A flared grip heel and narrowed neck rather than a rectangular block.
    hexa(
        &mut s,
        [
            Vec3::new(-0.014, 0.0, -0.020),
            Vec3::new(0.014, 0.0, -0.020),
            Vec3::new(0.010, 0.090, -0.012),
            Vec3::new(-0.010, 0.090, -0.012),
        ],
        [
            Vec3::new(-0.014, 0.0, 0.020),
            Vec3::new(0.014, 0.0, 0.020),
            Vec3::new(0.010, 0.090, 0.016),
            Vec3::new(-0.010, 0.090, 0.016),
        ],
        grain,
    );
    boxed(
        &mut s,
        Vec3::new(0.0, 0.105, 0.008),
        Vec3::new(0.013, 0.018, 0.030),
        steel,
    );
    turned(
        &mut s,
        Vec3::new(0.0, 0.0, 0.020),
        &[
            (0.124, 0.0),
            (0.124, 0.018),
            (0.128, 0.021),
            (0.172, 0.021),
            (0.176, 0.018),
            (0.176, 0.0),
        ],
        steel,
    );
    turned(
        &mut s,
        Vec3::new(0.0, 0.0, 0.030),
        &[
            (0.168, 0.0),
            (0.168, 0.011),
            (0.250, 0.009),
            (0.261, 0.009),
            (0.261, 0.005),
            (0.247, 0.005),
            (0.247, 0.0),
        ],
        steel,
    );
    boxed(
        &mut s,
        Vec3::new(0.0, 0.251, 0.042),
        Vec3::new(0.003, 0.007, 0.004),
        steel,
    );
    boxed(
        &mut s,
        Vec3::new(0.0, 0.125, -0.014),
        Vec3::new(0.005, 0.012, 0.008),
        steel,
    );
    // Three sides of the guard; the frame closes the fourth. Open air around
    // the curved trigger matters more at this scale than engraving.
    for (c, h) in [
        (
            Vec3::new(0.0, 0.132, -0.024),
            Vec3::new(0.004, 0.003, 0.018),
        ),
        (
            Vec3::new(0.0, 0.116, -0.040),
            Vec3::new(0.004, 0.016, 0.003),
        ),
        (
            Vec3::new(0.0, 0.101, -0.030),
            Vec3::new(0.004, 0.003, 0.010),
        ),
        (
            Vec3::new(0.0, 0.115, -0.025),
            Vec3::new(0.003, 0.006, 0.003),
        ),
    ] {
        boxed(&mut s, c, h, steel);
    }
    s.mesh()
}

/// The mesh behind a `HeldSrc::Gen` name.
///
/// **A name with no arm here is a panic at boot**, on purpose: the row was
/// written pointing at nothing, and an empty hand that looks like a missing
/// model is the silent failure `viewmodel::spawn_item`'s comment already
/// paid a capture to find. `tests/held_assets.rs` builds every `Gen` row, so
/// CI reaches this before a boot does.
pub fn mesh(name: &str) -> Mesh {
    match name {
        "torch" => torch_mesh(),
        "revolver" => revolver_mesh(),
        _ => panic!("HeldSrc::Gen({name:?}) has no generator in render::heldgen"),
    }
}

/// The material behind a `HeldSrc::Gen` name. One per row (`viewmodel::
/// Models` pairs them by index); the mesh's vertex tints carry the part
/// break-up, so wood and steel share whichever response fits the row's
/// dominant read. Emissive stays at its default — black — and
/// `tests/held_assets.rs` asserts that, for the torch too: what a lit row
/// carries is a `PointLight` on the hand, never a bright material.
pub fn material(name: &str) -> StandardMaterial {
    match name {
        "torch" => StandardMaterial {
            perceptual_roughness: 0.85,
            // See `render::fresnel`: 0.14 was F0 0.31%.
            reflectance: super::fresnel::DIELECTRIC,
            ..default()
        },
        // The stand-in head's lesson restated: not a mirror — a near-specular
        // metal with no reflection probe has nothing to reflect but sky.
        "revolver" => StandardMaterial {
            perceptual_roughness: 0.52,
            metallic: 0.55,
            reflectance: super::fresnel::METAL_DIELECTRIC,
            ..default()
        },
        _ => panic!("HeldSrc::Gen({name:?}) has no material in render::heldgen"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_generated_mesh_carries_uvs_and_tangents() {
        // A normal map without tangents is silently ignored — the failure that
        // looks like "the texture did not load" and reports nothing. Only the
        // stand-in handle actually wears a map today, but the four meshes
        // share one emitter and one law is cheaper than remembering which
        // rows are allowed to regress.
        for m in [handle_mesh(), head_mesh(), mesh("torch"), mesh("revolver")] {
            assert!(m.attribute(Mesh::ATTRIBUTE_UV_0).is_some());
            assert!(m.attribute(Mesh::ATTRIBUTE_TANGENT).is_some());
        }
    }
}
