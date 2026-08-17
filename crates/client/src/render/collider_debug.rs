//! Draw what the SIM blocks, on top of what the client draws — F3.
//!
//! **This exists because a mismatch between the two was diagnosed by reading
//! code for an hour when looking at it would have taken a second.** The sim's
//! occupant volumes (`terrain::OCCUPANT_R_M` / `OCCUPANT_TOP_M`) are numbers
//! in a table, and the meshes they are supposed to describe are generated;
//! nothing in a frame shows where one stops. The `Tree` row spent the life of
//! the native client at 0.26 m — the bottom radius of a `CylinderGeometry`
//! from the deleted browser client — against a trunk that measures 0.2398 m,
//! and the report that found it was a player saying the bark looked offset
//! from the thing that stopped them.
//!
//! `CLAUDE.md` forbids building a replacement pixel gate and this is not one:
//! it asserts nothing, scores nothing, and runs only when a person presses a
//! key. It is the *other* half of "the operator boots the game and looks" —
//! making the invisible thing visible so that looking is enough.
//!
//! It also draws the player's own capsule at `predict.position()` — the
//! sim-truth body, NOT `render_position()` — so the reconciliation offset is
//! visible as a gap between the capsule and the camera rather than as a
//! feeling. That gap was a real defect (`ClientCore::advance`) and had no
//! surface until now.

use bevy::prelude::*;
use sim_core::gather::cell_key;
use sim_core::terrain::{self, Occupant};

use super::{Eye, Net, WorldId};

/// Is the overlay up? Off by default and never persisted — a debug view that
/// survived a restart would eventually ship in a screenshot.
#[derive(Resource, Default)]
pub struct ShowColliders(pub bool);

/// How far out to draw, in scatter cells. Four cells is 32 m, which covers
/// everything a player can walk into before the next frame and keeps the
/// gizmo count low enough not to change what it is measuring.
const CELLS: i32 = 4;

/// Segments per circle. Twelve reads as a circle at these radii and costs a
/// twelfth of what a smooth one would.
const SEGMENTS: usize = 12;

/// F3 toggles. No F-key was bound before this one.
pub fn toggle(keys: Res<ButtonInput<KeyCode>>, mut show: ResMut<ShowColliders>) {
    if keys.just_pressed(KeyCode::F3) {
        show.0 = !show.0;
    }
}

/// One horizontal ring at height `y`, centred on (`x`, `z`).
fn ring(gizmos: &mut Gizmos, x: f32, y: f32, z: f32, r: f32, color: Color) {
    let pts = (0..=SEGMENTS).map(|i| {
        let a = i as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
        Vec3::new(x + r * a.cos(), y, z + r * a.sin())
    });
    gizmos.linestrip(pts, color);
}

/// A vertical cylinder as two rings and four uprights — enough to read the
/// footprint AND the height, which is the pair `slot_blocks` actually tests.
fn cylinder(gizmos: &mut Gizmos, x: f32, y0: f32, y1: f32, z: f32, r: f32, color: Color) {
    ring(gizmos, x, y0, z, r, color);
    ring(gizmos, x, y1, z, r, color);
    for i in 0..4 {
        let a = i as f32 / 4.0 * std::f32::consts::TAU;
        let (dx, dz) = (r * a.cos(), r * a.sin());
        gizmos.line(
            Vec3::new(x + dx, y0, z + dz),
            Vec3::new(x + dx, y1, z + dz),
            color,
        );
    }
}

/// Draw the blocking volume of every scattered occupant near the eye, plus
/// the player's own capsule footprint.
pub fn draw(
    show: Res<ShowColliders>,
    eye: Res<Eye>,
    world: Res<WorldId>,
    net: NonSend<Net>,
    mut gizmos: Gizmos,
) {
    if !show.0 {
        return;
    }
    let core = &net.session.core;

    // The occupants. Yellow for what stops you; the mesh is whatever the
    // renderer already drew, so any daylight between the two is the defect
    // this view exists to show.
    let ecx = (eye.pos.x / terrain::CELL_SIZE).floor() as i32;
    let ecz = (eye.pos.z / terrain::CELL_SIZE).floor() as i32;
    for dz in -CELLS..=CELLS {
        for dx in -CELLS..=CELLS {
            let (cx, cz) = (ecx + dx, ecz + dz);
            if cx < 0 || cz < 0 || cx >= terrain::CELLS_PER_SIDE || cz >= terrain::CELLS_PER_SIDE {
                continue;
            }
            let slot = terrain::scatter(world.seed, &world.table, &world.haven, cx, cz);
            if slot.occupant == Occupant::None {
                continue;
            }
            // A harvested slot blocks nothing, and drawing it would report a
            // wall where the sim has none — the exact inverse of the mistake
            // this view is for.
            if core.harvested.contains(cell_key(cx as u16, cz as u16)) {
                continue;
            }
            let (r, top) = terrain::occupant_volume(slot.occupant);
            if r <= 0.0 {
                continue;
            }
            // The two box-shaped occupants publish a BROAD-PHASE radius, not
            // their volume (`OCCUPANT_R_M` rows 10 and 12), so the ring drawn
            // for them is honestly wider than what stops a body. Dimmer, so
            // the difference is visible rather than misread as a fat collider.
            let broad = matches!(
                slot.occupant,
                Occupant::HavenShelter | Occupant::WaystationCanopy
            );
            let color = if broad {
                Color::srgba(0.9, 0.75, 0.2, 0.35)
            } else {
                Color::srgba(1.0, 0.85, 0.1, 0.9)
            };
            cylinder(
                &mut gizmos,
                slot.x,
                slot.y,
                slot.y + top * slot.scale,
                slot.z,
                r * slot.scale,
                color,
            );
        }
    }

    // The player, at the body the sim owns rather than the one the camera is
    // smoothed to. When these separate, the world will look offset — which is
    // precisely the symptom that led here.
    let [px, py, pz] = core.predict.position();
    cylinder(
        &mut gizmos,
        px,
        py,
        py + sim_core::collide::CAPSULE_HEIGHT_M,
        pz,
        sim_core::collide::CAPSULE_RADIUS_M,
        Color::srgba(0.3, 0.9, 1.0, 0.9),
    );
    // And the smoothing offset itself, as a line from the body to the camera.
    // Zero-length in the healthy case, which is the point: anything you can
    // see here is the reconciliation offset failing to drain.
    let [rx, ry, rz] = core.predict.render_position();
    gizmos.line(
        Vec3::new(px, py, pz),
        Vec3::new(rx, ry, rz),
        Color::srgba(1.0, 0.2, 0.2, 1.0),
    );
}
