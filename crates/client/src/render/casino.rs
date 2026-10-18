//! THE GATE's casino, drawn (`sim_core::casino`, `town::CASINO`): Rust's big
//! wheel on its stand with the betting terminals before it, and the slot
//! machines' cabinets. The boxes are the sim's collision boxes; the wheel is a
//! disc of Rust's twenty-five coloured segments that turns when the shard
//! says it spins (`EventMsg::WheelSpin`) and comes to rest on the segment it
//! drew, on the shard's clock, so every screen in the yard sees it stop
//! together.

use std::f32::consts::TAU;

use bevy::prelude::*;
use sim_core::casino::{WHEEL, WHEEL_OPEN_TICKS, WHEEL_ROUND_TICKS, WHEEL_SEGMENTS};
use sim_core::town::{CASINO, SLOT_SPOTS, WHEEL_HUB, WHEEL_RADIUS_M, WHEEL_TERMINALS};

use super::props::{boxes_mesh_with, linear, Soup};
use super::{Net, WorldEntity, WorldId};

/// Whole turns the wheel makes before it settles.
const TURNS: f32 = 5.0;
/// How far in front of its stand the disc hangs, metres.
const DISC_OFF_M: f32 = 0.12;

const STAND: u32 = 0x3b3029;
const TRIM: u32 = 0xc9a24a;
const TERMINAL: u32 = 0x4a4f55;
const SCREEN: u32 = 0x2f6f62;
const CABINET: u32 = 0x7a1f1f;
const GLASS: u32 = 0xe8e2d0;
const LAMP: u32 = 0xf2c230;
const POINTER: u32 = 0xd8d4c8;

#[derive(Component)]
pub struct CasinoVisual;

/// The turning disc: its rest transform, and the spin it is drawing (the
/// round, the angle it started from and the one it stops at).
#[derive(Component)]
pub struct WheelDisc {
    base: Transform,
    round: Option<u32>,
    from: f32,
    to: f32,
}

type Boxes = Vec<([f32; 3], [f32; 3], u32)>;

fn put(out: &mut Boxes, b: [f32; 6], hex: u32) {
    out.push((
        [
            (b[0] + b[3]) * 0.5,
            (b[1] + b[4]) * 0.5,
            (b[2] + b[5]) * 0.5,
        ],
        [
            (b[3] - b[0]) * 0.5,
            (b[4] - b[1]) * 0.5,
            (b[5] - b[2]) * 0.5,
        ],
        hex,
    ));
}

/// Everything that stands still, in the town's frame.
fn fixed_boxes() -> Boxes {
    let mut out = Boxes::new();
    let s = CASINO[0].b;
    put(&mut out, s, STAND);
    // A gilt cap on the stand, and the pointer over the top of the disc.
    put(
        &mut out,
        [
            s[0] - 0.05,
            s[4],
            s[2] - 0.05,
            s[3] + 0.05,
            s[4] + 0.12,
            s[5] + 0.05,
        ],
        TRIM,
    );
    let (hx, hy, hz) = WHEEL_HUB;
    let top = hy + WHEEL_RADIUS_M;
    put(
        &mut out,
        [
            hx - DISC_OFF_M - 0.12,
            top - 0.18,
            hz - 0.06,
            hx,
            top + 0.12,
            hz + 0.06,
        ],
        POINTER,
    );
    for k in 0..WHEEL_TERMINALS.len() {
        let b = CASINO[1 + k].b;
        put(&mut out, b, TERMINAL);
        put(
            &mut out,
            [
                b[0] + 0.05,
                b[4],
                b[2] + 0.05,
                b[3] - 0.05,
                b[4] + 0.04,
                b[5] - 0.05,
            ],
            SCREEN,
        );
    }
    for k in 0..SLOT_SPOTS.len() {
        let b = CASINO[1 + WHEEL_TERMINALS.len() + k].b;
        put(&mut out, b, CABINET);
        // The reel window on the face toward the player, the lamp on top, and
        // the lever on the side.
        put(
            &mut out,
            [b[0] + 0.08, 1.05, b[2] - 0.02, b[3] - 0.08, 1.45, b[2]],
            GLASS,
        );
        put(
            &mut out,
            [
                b[0] + 0.15,
                b[4],
                b[2] + 0.2,
                b[3] - 0.15,
                b[4] + 0.12,
                b[5] - 0.2,
            ],
            LAMP,
        );
        put(
            &mut out,
            [b[3], 1.0, b[2] + 0.3, b[3] + 0.06, 1.6, b[2] + 0.36],
            TRIM,
        );
    }
    out
}

/// The disc in its own frame: facing -X, segment `i` centred at angle
/// `i × 2π/25` from straight up, toward +Z. A dark back plate shows between
/// the segments and round the rim.
fn disc_mesh() -> Mesh {
    let mut s = Soup::tiling(1.0);
    let r = WHEEL_RADIUS_M;
    let at = |phi: f32, rad: f32, x: f32| Vec3::new(x, rad * phi.cos(), rad * phi.sin());
    let w = TAU / WHEEL_SEGMENTS as f32;
    let flat = |c: [f32; 3]| move |_: Vec3| [c[0], c[1], c[2], 1.0];
    // The back plate, a 64-gon.
    let back = flat(linear(STAND));
    for i in 0..64 {
        let (a, b) = (i as f32 * TAU / 64.0, (i + 1) as f32 * TAU / 64.0);
        s.tri(Vec3::ZERO, at(b, r, 0.0), at(a, r, 0.0), back, None, 0.0);
    }
    // The segments, a hair in front and inset from the rim.
    let gap = 0.008;
    for (i, &c) in WHEEL.iter().enumerate() {
        let [cr, cg, cb] = super::panels::casino::COLOURS[c as usize].1;
        let col = flat([cr.powf(2.2), cg.powf(2.2), cb.powf(2.2)]);
        let (a0, a1) = (i as f32 * w - w * 0.5 + gap, i as f32 * w + w * 0.5 - gap);
        for k in 0..3 {
            let a = a0 + (a1 - a0) * k as f32 / 3.0;
            let b = a0 + (a1 - a0) * (k + 1) as f32 / 3.0;
            let (ri, ro) = (0.3, r - 0.1);
            let (p0, p1) = (at(a, ri, -0.02), at(b, ri, -0.02));
            let (q0, q1) = (at(a, ro, -0.02), at(b, ro, -0.02));
            s.tri(p0, q1, q0, col, None, 0.0);
            s.tri(p0, p1, q1, col, None, 0.0);
        }
    }
    // The gilt hub.
    let hub = flat(linear(TRIM));
    for i in 0..24 {
        let (a, b) = (i as f32 * TAU / 24.0, (i + 1) as f32 * TAU / 24.0);
        let c = Vec3::new(-0.05, 0.0, 0.0);
        s.tri(c, at(b, 0.32, -0.05), at(a, 0.32, -0.05), hub, None, 0.0);
    }
    s.mesh()
}

/// The disc's angle with segment `seg` under the pointer (straight up).
fn rest_angle(seg: u8) -> f32 {
    -(seg as f32) * TAU / WHEEL_SEGMENTS as f32
}

pub fn spawn(
    mut commands: Commands,
    world: Option<Res<WorldId>>,
    drawn: Query<(), With<CasinoVisual>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let Some(world) = world else { return };
    let t = world.haven.town;
    if !t.live || !drawn.is_empty() {
        return;
    }
    let tf = super::town::transform(&t);
    let mat = materials.add(StandardMaterial {
        base_color: Color::WHITE,
        perceptual_roughness: 0.6,
        ..default()
    });
    commands.spawn((
        WorldEntity,
        CasinoVisual,
        Mesh3d(meshes.add(boxes_mesh_with(&fixed_boxes(), linear, 1.0))),
        MeshMaterial3d(mat.clone()),
        tf,
    ));
    // The disc catches the yard's lamps a little, so it reads at night.
    let lit = materials.add(StandardMaterial {
        base_color: Color::WHITE,
        perceptual_roughness: 0.45,
        emissive: LinearRgba::rgb(0.08, 0.07, 0.05),
        ..default()
    });
    let (hx, hy, hz) = WHEEL_HUB;
    let base = tf.mul_transform(Transform::from_xyz(hx - DISC_OFF_M, hy, hz));
    commands.spawn((
        WorldEntity,
        CasinoVisual,
        WheelDisc {
            base,
            round: None,
            from: 0.0,
            to: 0.0,
        },
        Mesh3d(meshes.add(disc_mesh())),
        MeshMaterial3d(lit),
        base,
    ));
}

/// Turn the disc: still between spins, and through a spin from where it
/// rested to `TURNS` turns and the drawn segment, slowing as Rust's does.
pub fn spin(net: NonSend<Net>, mut q: Query<(&mut WheelDisc, &mut Transform)>) {
    let core = &net.session.core;
    let now = core.server_tick_est().max(0.0);
    for (mut d, mut tf) in q.iter_mut() {
        let mut angle = d.to;
        if let Some((round, seg)) = core.casino.spin {
            if d.round != Some(round) {
                // A new spin starts from wherever the disc stands.
                let from = d.to.rem_euclid(TAU);
                let land = from - TURNS * TAU;
                let to = land - (land - rest_angle(seg)).rem_euclid(TAU);
                *d = WheelDisc {
                    base: d.base,
                    round: Some(round),
                    from,
                    to,
                };
            }
            let start = round as f64 * WHEEL_ROUND_TICKS as f64 + WHEEL_OPEN_TICKS as f64;
            let len = (WHEEL_ROUND_TICKS - WHEEL_OPEN_TICKS - 1) as f64;
            let u = ((now - start) / len).clamp(0.0, 1.0) as f32;
            let ease = 1.0 - (1.0 - u).powi(3);
            angle = d.from + (d.to - d.from) * ease;
        }
        let want = d
            .base
            .mul_transform(Transform::from_rotation(Quat::from_rotation_x(angle)));
        if *tf != want {
            *tf = want;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A segment at rest sits under the pointer: rotating its centre by its
    /// rest angle brings it straight up.
    #[test]
    fn a_resting_segment_is_under_the_pointer() {
        for seg in 0..WHEEL_SEGMENTS as u8 {
            let phi = seg as f32 * TAU / WHEEL_SEGMENTS as f32;
            let p = Vec3::new(0.0, phi.cos(), phi.sin());
            let up = Quat::from_rotation_x(rest_angle(seg)) * p;
            assert!(
                (up - Vec3::Y).length() < 1e-4,
                "segment {seg} rests at {up}"
            );
        }
    }
}
