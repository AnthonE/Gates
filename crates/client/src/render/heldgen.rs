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
//! `tests/held_assets.rs` holds a generated row's measured length to its
//! declared `length_m` exactly as it holds a file's.
//!
//! Nothing here glows: a carried light source is forbidden by name
//! (`tests/held_assets.rs::nothing_held_glows`), and the torch is an inert
//! prop until the flashlight question is answered (`NOW.md` §0pvp item 3).

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

/// The torch: a wrapped head on a stick, authored standing so it is carried
/// upright (`lay_forward: false` on its row). No flame and no glow — the
/// item is inert (`NOW.md` §0pvp item 3) and `nothing_held_glows` is the
/// gate that keeps a carried emissive out.
fn torch_mesh() -> Mesh {
    let mut s = Soup::tiling(3.0);
    // Absolute albedos, NOT `tint1`: the stand-in's tints are mean-1 because
    // they multiply a photograph (the handle) or a material whose
    // `base_color` carries the value (the head). These two meshes pair with
    // a WHITE-based material and no map, so a mean-1 tint renders wood as
    // cream — the vertex colour is the whole albedo here and it says so.
    let grain = [0.30, 0.23, 0.15];
    // The stick. Slightly slimmer than the stand-in's haft: this one is a
    // branch, not a tool handle.
    boxed(
        &mut s,
        Vec3::new(0.0, 0.19, 0.0),
        Vec3::new(0.013, 0.19, 0.013),
        grain,
    );
    // The wrap: rag wound over the head, darker and fatter than the stick,
    // with a second turn offset so the silhouette is a bundle rather than a
    // block.
    let rag = [0.16, 0.14, 0.12];
    boxed(
        &mut s,
        Vec3::new(0.0, 0.410, 0.0),
        Vec3::new(0.028, 0.045, 0.028),
        rag,
    );
    boxed(
        &mut s,
        Vec3::new(0.004, 0.372, -0.004),
        Vec3::new(0.022, 0.016, 0.022),
        rag,
    );
    s.mesh()
}

/// The revolver, authored BARREL-UP so the table's shared quarter-turn
/// (`lay_forward: true`) points it forward: authored +Y is held −Z (away),
/// authored +Z is held +Y (up), so height here is depth in hand and the bore
/// axis is the column. Grip low (its row's `grip_frac`), bore ~3 cm above
/// the fist, the way a revolver actually sits.
fn revolver_mesh() -> Mesh {
    let mut s = Soup::tiling(1.0);
    // Absolute albedos — `torch_mesh` says why these are not `tint1`.
    let steel = [0.29, 0.30, 0.33];
    let grain = [0.24, 0.16, 0.10];
    // The handle the fist closes on. Wood.
    boxed(
        &mut s,
        Vec3::new(0.0, 0.045, 0.0),
        Vec3::new(0.014, 0.045, 0.020),
        grain,
    );
    // The frame over it.
    boxed(
        &mut s,
        Vec3::new(0.0, 0.105, 0.008),
        Vec3::new(0.013, 0.018, 0.030),
        steel,
    );
    // The cylinder: the one bulge that says revolver rather than pistol.
    boxed(
        &mut s,
        Vec3::new(0.0, 0.150, 0.020),
        Vec3::new(0.017, 0.026, 0.021),
        steel,
    );
    // The barrel, a slim column to the muzzle, bore forward of the cylinder
    // axis (authored +Z = held up).
    boxed(
        &mut s,
        Vec3::new(0.0, 0.213, 0.030),
        Vec3::new(0.009, 0.048, 0.011),
        steel,
    );
    // The hammer spur, behind the cylinder (authored −Z = held down is
    // wrong for a spur — it rides the TOP of the frame, held toward the
    // eye, which authored is −Y... a spur this size reads from silhouette
    // alone, so it sits where the silhouette wants it: aft of the cylinder).
    boxed(
        &mut s,
        Vec3::new(0.0, 0.125, -0.014),
        Vec3::new(0.005, 0.012, 0.008),
        steel,
    );
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
/// `tests/held_assets.rs` asserts that.
pub fn material(name: &str) -> StandardMaterial {
    match name {
        "torch" => StandardMaterial {
            perceptual_roughness: 0.85,
            reflectance: 0.14,
            ..default()
        },
        // The stand-in head's lesson restated: not a mirror — a near-specular
        // metal with no reflection probe has nothing to reflect but sky.
        "revolver" => StandardMaterial {
            perceptual_roughness: 0.52,
            metallic: 0.55,
            reflectance: 0.35,
            ..default()
        },
        _ => panic!("HeldSrc::Gen({name:?}) has no material in render::heldgen"),
    }
}
