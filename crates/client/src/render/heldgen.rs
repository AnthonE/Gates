//! Generated held-item geometry: the meshes behind `ui::hold::HeldSrc::Gen`
//! rows.
//!
//! **Why generated rather than sourced.** The revolver, torch, crossbow and
//! bat were built here until their `.glb`s landed (2026-10-24,
//! `MANIFEST.md`). The bat's lathe stays: it is the canvas a painted bat
//! skin draws on ([`canvas_mesh`]), whose UVs are a public contract
//! (`render/canvas.rs`), while the plain bat draws its file. When a real
//! asset lands, the row in `HELD_MODELS` flips from `Gen` to `Glb`;
//! nothing else moves.
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

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;

use super::props::Soup;

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

/// Revolve a profile around +Y with SMOOTH normals. `turned` facets every
/// band, which suits a torch's wrap and not a bat, whose whole read is one
/// polished taper. `sides + 1` columns so the UV seam closes; a radius of
/// zero is a pole, and the degenerate half of each quad touching it is
/// skipped (mikktspace refuses a zero-area triangle).
fn lathe(profile: &[(f32, f32)], sides: usize, color: impl Fn(f32, f32, f32) -> [f32; 4]) -> Mesh {
    let n = profile.len();
    // V by arc length down the profile, tip at the top of the image (0) and
    // the foot at the bottom (1), so a painted skin's knob and dome get the
    // room their surface has. **A public contract** (`DECISIONS.md:432`):
    // `tests/skin_templates.rs` pins it.
    let mut arc = Vec::with_capacity(n);
    let mut run = 0.0f32;
    for k in 0..n {
        if k > 0 {
            let (y0, r0) = profile[k - 1];
            let (y1, r1) = profile[k];
            run += ((y1 - y0).powi(2) + (r1 - r0).powi(2)).sqrt();
        }
        arc.push(run);
    }
    let total = run.max(1e-9);
    let cols = sides + 1;
    let mut pos = Vec::with_capacity(n * cols);
    let mut nrm = Vec::with_capacity(n * cols);
    let mut uv = Vec::with_capacity(n * cols);
    let mut col = Vec::with_capacity(n * cols);
    for (k, &(y, r)) in profile.iter().enumerate() {
        // The profile's outward normal in the (r, y) half-plane, from its
        // neighbours: a central difference, one-sided at the two ends.
        let (ya, ra) = profile[k.saturating_sub(1)];
        let (yb, rb) = profile[(k + 1).min(n - 1)];
        let (dr, dy) = (rb - ra, yb - ya);
        let len = (dr * dr + dy * dy).sqrt().max(1e-9);
        let (nr, ny) = (dy / len, -dr / len);
        for j in 0..cols {
            let a = j as f32 * std::f32::consts::TAU / sides as f32;
            let (sin, cos) = a.sin_cos();
            pos.push([cos * r, y, sin * r]);
            nrm.push([cos * nr, ny, sin * nr]);
            uv.push([j as f32 / sides as f32, 1.0 - arc[k] / total]);
            col.push(color(y, r, a));
        }
    }
    let w = cols as u32;
    let mut idx = Vec::with_capacity((n - 1) * sides * 6);
    for k in 0..n - 1 {
        let (r0, r1) = (profile[k].1, profile[k + 1].1);
        for j in 0..sides as u32 {
            let a = k as u32 * w + j;
            let b = a + w;
            // `turned`'s winding: ring k, ring k+1, ring k+1 one column on.
            if r1 > 0.0 {
                idx.extend([a, b, b + 1]);
            }
            if r0 > 0.0 {
                idx.extend([a, b + 1, a + 1]);
            }
        }
    }
    let mut m = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD | RenderAssetUsages::MAIN_WORLD,
    );
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
    m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, nrm);
    m.insert_attribute(Mesh::ATTRIBUTE_COLOR, col);
    m.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv);
    m.insert_indices(Indices::U32(idx));
    m.generate_tangents()
        .expect("a lathe has positions, normals and UVs");
    m
}

/// The bat's +Y extent: the tip of the dome. `ui::hold`'s row restates it.
const BAT_LEN: f32 = 0.840;

/// Triple T's bat (operator, 2026-10-06: *"kind of fatter"*): the club in the
/// original Tung Tung Tung Sahur picture, measured off it rather than off a
/// regulation bat. Against a real wood bat it has a handle ~40% thicker
/// (3.4 cm), a barrel ~30% fatter (8.8 cm), a taper that starts at the hand
/// and keeps going instead of a long thin handle, and a big rounded dome for
/// an end. 84 cm, knob at the foot, so the fist closes just above the knob.
fn bat_mesh() -> Mesh {
    const HANDLE_R: f32 = 0.017;
    const BARREL_R: f32 = 0.044;
    // The taper, handle to barrel, as a smoothstep over this span of y.
    const TAPER: (f32, f32) = (0.16, 0.74);
    // Where the barrel stops and the dome starts. The dome is a
    // superellipse quarter, blunter than a hemisphere and longer than one.
    const DOME: f32 = 0.790;
    const DOME_P: f32 = 2.2;
    // The knob, foot up: a flattened bead flaring into the handle.
    let mut p: Vec<(f32, f32)> = vec![
        (0.000, 0.0),
        (0.001, 0.013),
        (0.004, 0.020),
        (0.008, 0.0245),
        (0.013, 0.0260),
        (0.018, 0.0255),
        (0.023, 0.0225),
        (0.028, 0.0190),
        (0.034, 0.0172),
    ];
    let steps = 48;
    for i in 1..=steps {
        let y = 0.034 + (DOME - 0.034) * i as f32 / steps as f32;
        let t = ((y - TAPER.0) / (TAPER.1 - TAPER.0)).clamp(0.0, 1.0);
        p.push((
            y,
            HANDLE_R + (BARREL_R - HANDLE_R) * t * t * (3.0 - 2.0 * t),
        ));
    }
    let h = BAT_LEN - DOME;
    for i in 1..=12 {
        let a = i as f32 / 12.0;
        let r = if i == 12 {
            0.0
        } else {
            BARREL_R * (1.0 - a.powf(DOME_P)).powf(1.0 / DOME_P)
        };
        p.push((DOME + h * a, r));
    }

    // Honey-amber varnished wood, the picture's colour taken out of its
    // lamplight. The grain is strong on purpose: a vertex tint is all this
    // mesh has for a surface, and at the first cut's ±5% the in-game frame
    // read flat peach plastic (measured ±4 of 255 along the whole bat).
    // Long streaks with thin dark lines between them, darker where hands
    // have been.
    const WOOD: [f32; 3] = [0.58, 0.21, 0.025];
    lathe(&p, 40, |y, _r, a| {
        let streak = 0.12 * (7.0 * a + 2.2 * (3.0 * a + 6.0 * y).sin()).sin()
            + 0.08 * (17.0 * a + 4.0 * y).sin();
        let line = (11.0 * a + 1.3 * (5.0 * y + 2.0 * a).sin()).sin().max(0.0);
        let along = 0.05 * (13.0 * y).sin();
        let grain = 1.0 + streak + along - 0.30 * line.powi(8);
        let handled = if y < 0.30 {
            0.72 + 0.28 * (y / 0.30)
        } else {
            1.0
        };
        let k = grain * handled;
        [WOOD[0] * k, WOOD[1] * k, WOOD[2] * k, 1.0]
    })
}

/// An arrow `len` long up +Y, nock at the origin and point at the far end:
/// the shaft, a steel point and three vanes, one of them the cock feather.
/// The colours are the crossbow bolt's, so the two read as one family. Drawn
/// on the string of the bow in your own hand (`render::bow`).
pub fn arrow_mesh(len: f32) -> Mesh {
    let mut s = Soup::tiling(1.0);
    let pale = [0.52, 0.40, 0.26];
    let nock = [0.17, 0.12, 0.08];
    let steel = [0.29, 0.30, 0.33];
    let vane = [0.45, 0.13, 0.10];
    let hen = [0.36, 0.32, 0.26];
    let (r, head) = (0.0045, 0.065);
    turned(
        &mut s,
        Vec3::ZERO,
        &[(0.0, 0.0), (0.0, 0.0055), (0.014, 0.0055), (0.014, r)],
        nock,
    );
    turned(&mut s, Vec3::ZERO, &[(0.014, r), (len - head, r)], pale);
    turned(
        &mut s,
        Vec3::ZERO,
        &[(len - head, r), (len - head + 0.012, 0.012), (len, 0.0)],
        steel,
    );
    // The vanes, both faces each: a plate standing off the shaft, cut back
    // toward the nock.
    for (i, tint) in [vane, hen, hen].into_iter().enumerate() {
        let a = std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::TAU / 3.0;
        let out = Vec3::new(a.cos(), 0.0, a.sin());
        let at = |y: f32, h: f32| Vec3::Y * y + out * (r + h);
        let q = [
            at(0.025, 0.0),
            at(0.105, 0.0),
            at(0.10, 0.003),
            at(0.04, 0.010),
        ];
        let col = move |_: Vec3| [tint[0], tint[1], tint[2], 1.0];
        for (x, y, z) in [(0, 1, 2), (0, 2, 3), (0, 2, 1), (0, 3, 2)] {
            s.tri(q[x], q[y], q[z], col, None, 0.0);
        }
    }
    s.mesh()
}

/// The arrow's surface: wood, mostly. See [`arrow_mesh`].
pub fn arrow_material() -> StandardMaterial {
    StandardMaterial {
        perceptual_roughness: 0.72,
        reflectance: super::fresnel::DIELECTRIC,
        ..default()
    }
}

/// The mesh behind a `HeldSrc::Gen` name.
///
/// **A name with no arm here is a panic at boot**, on purpose: the row was
/// written pointing at nothing, and an empty hand that looks like a missing
/// model is the silent failure `viewmodel::spawn_item`'s comment already
/// paid a capture to find. `tests/held_assets.rs` builds every `Gen` row, so
/// CI reaches this before a boot does.
/// The mesh a painted skin draws on (`ui::skins::CANVASES`): the row's own
/// shape with its vertex tints taken off, so the painted texture is the
/// whole of its colour. `None` for a row nobody can paint yet.
pub fn canvas_mesh(name: &str) -> Option<Mesh> {
    match name {
        "bat" => {
            let mut m = bat_mesh();
            m.remove_attribute(Mesh::ATTRIBUTE_COLOR);
            Some(m)
        }
        _ => None,
    }
}

/// Whether a `HeldSrc::Gen` name is the prop kit's (`render::heldkit`) rather
/// than the bat built here.
pub fn is_kit(name: &str) -> bool {
    name != "bat" && super::heldkit::NAMES.contains(&name)
}

pub fn mesh(name: &str) -> Mesh {
    match name {
        "bat" => bat_mesh(),
        // Everything else with no model is the prop kit's.
        _ => super::heldkit::baked(name).unwrap_or_else(|| {
            panic!("HeldSrc::Gen({name:?}) has no generator in render::heldgen")
        }),
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
        // Varnished: the picture's bat carries a clear highlight down its
        // length, so much smoother than the torch's raw shaft.
        "bat" => StandardMaterial {
            perceptual_roughness: 0.38,
            reflectance: super::fresnel::DIELECTRIC,
            ..default()
        },
        _ if is_kit(name) => super::heldkit::material(),
        _ => panic!("HeldSrc::Gen({name:?}) has no material in render::heldgen"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_generated_mesh_carries_uvs_and_tangents() {
        // A normal map without tangents is silently ignored — the failure that
        // looks like "the texture did not load" and reports nothing. The
        // meshes share one emitter and one law is cheaper than remembering
        // which rows are allowed to regress.
        for m in [mesh("bat"), arrow_mesh(0.8)] {
            assert!(m.attribute(Mesh::ATTRIBUTE_UV_0).is_some());
            assert!(m.attribute(Mesh::ATTRIBUTE_TANGENT).is_some());
        }
    }
}
