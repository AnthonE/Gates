//! The attack helicopter — the client half (`sim-core/src/heli.rs`).
//!
//! On the wire it is the roster's last slot (`heli::HELI_SLOT`), a mob
//! record like a pig, so it arrives through the same interpolator. This file
//! takes that one id away from `mobs::stream` and draws a gunship instead:
//! a box-massed hull, a main rotor and a tail rotor that spin, and a bank
//! into whatever way it is moving. Nothing here decides anything — position,
//! heading and life are sim state; the spin and the bank are derived.
//!
//! **The origin is the feet**, like every body: the sim's gun is
//! `ranged::ARROW_EYE_MM` above it, which is where the nose gun sits here
//! and where `fx/gun.rs` draws the round from.

use bevy::prelude::*;

use super::props::{boxes_mesh_with, linear};
use super::Net;
use sim_core::heli::HELI_SLOT;
use sim_core::mob;

/// Is this wire id the helicopter?
#[inline]
pub fn is_heli(id: u32) -> bool {
    mob::slot_of_id(id) == Some(HELI_SLOT)
}

const OLIVE: u32 = 0x3d4430;
const DARK: u32 = 0x2a2f22;
const GLASS: u32 = 0x1c2630;
const STEEL: u32 = 0x1e1e1e;

/// The hull, facing **+Z** (the sim's yaw 0), skids at y = 0. Sized after a
/// light attack helicopter: ~13 m nose to tail, a 12 m rotor.
/// `(centre, half-extent, hex)`.
const HULL: &[([f32; 3], [f32; 3], u32)] = &[
    // Cabin — the silhouette.
    ([0.0, 2.2, 0.6], [1.0, 0.95, 2.4], OLIVE),
    // Nose, lower and narrower.
    ([0.0, 1.85, 3.5], [0.8, 0.65, 0.7], OLIVE),
    // Canopy.
    ([0.0, 2.7, 2.6], [0.82, 0.5, 1.0], GLASS),
    // Engine housing on the roof.
    ([0.0, 3.3, -0.3], [0.65, 0.32, 1.5], DARK),
    // Rotor mast.
    ([0.0, 3.75, -0.2], [0.12, 0.2, 0.12], STEEL),
    // Tail boom, fin, stabiliser.
    ([0.0, 2.55, -4.5], [0.28, 0.28, 3.2], OLIVE),
    ([0.0, 3.3, -7.6], [0.1, 0.85, 0.55], OLIVE),
    ([0.0, 2.6, -7.2], [1.15, 0.05, 0.32], OLIVE),
    // Stub wings and the pods under them.
    ([0.0, 1.9, 0.6], [1.75, 0.06, 0.38], DARK),
    ([-1.55, 1.6, 0.7], [0.24, 0.24, 0.85], DARK),
    ([1.55, 1.6, 0.7], [0.24, 0.24, 0.85], DARK),
    // The chin gun: its muzzle is [`MUZZLE`].
    ([0.0, 1.35, 3.95], [0.22, 0.22, 0.3], STEEL),
    ([0.0, 1.35, 4.6], [0.07, 0.07, 0.55], STEEL),
    // Skids and struts.
    ([-0.95, 0.08, 0.4], [0.07, 0.07, 2.3], STEEL),
    ([0.95, 0.08, 0.4], [0.07, 0.07, 2.3], STEEL),
    ([-0.85, 0.65, 1.5], [0.05, 0.55, 0.05], STEEL),
    ([0.85, 0.65, 1.5], [0.05, 0.55, 0.05], STEEL),
    ([-0.85, 0.65, -0.7], [0.05, 0.55, 0.05], STEEL),
    ([0.85, 0.65, -0.7], [0.05, 0.55, 0.05], STEEL),
];

/// The chin gun's muzzle, in hull space — where the flash goes.
pub const MUZZLE: Vec3 = Vec3::new(0.0, 1.35, 5.15);

/// Two blades crossed at the hub, in the rotor's own plane (XZ).
const ROTOR: &[([f32; 3], [f32; 3], u32)] = &[
    ([0.0, 0.0, 0.0], [6.0, 0.03, 0.16], STEEL),
    ([0.0, 0.0, 0.0], [0.16, 0.03, 6.0], STEEL),
    ([0.0, 0.0, 0.0], [0.25, 0.08, 0.25], STEEL),
];
const ROTOR_AT: Vec3 = Vec3::new(0.0, 3.98, -0.2);
/// The tail rotor, in the YZ plane, on the fin's left.
const TAIL_ROTOR: &[([f32; 3], [f32; 3], u32)] = &[
    ([0.0, 0.0, 0.0], [0.03, 1.05, 0.1], STEEL),
    ([0.0, 0.0, 0.0], [0.03, 0.1, 1.05], STEEL),
];
const TAIL_ROTOR_AT: Vec3 = Vec3::new(-0.16, 3.3, -7.7);

/// Rotor speeds, radians a second. Faster than any frame rate resolves,
/// which is the point: the blades read as a blur that turns.
const ROTOR_RAD_S: f32 = 31.0;
const TAIL_RAD_S: f32 = 57.0;

/// The bank: radians of tilt per m/s of horizontal speed, capped — a
/// helicopter leans into the way it is going.
const BANK_PER_MPS: f32 = 0.012;
const BANK_MAX: f32 = 0.3;
/// How fast the derived velocity settles, per second.
const VEL_SMOOTH: f32 = 4.0;

#[derive(Resource)]
pub struct HeliAssets {
    hull: Handle<Mesh>,
    rotor: Handle<Mesh>,
    tail: Handle<Mesh>,
    material: Handle<StandardMaterial>,
}

/// The drawn helicopter: its entity, and the velocity read off two
/// interpolated positions (the wire carries none).
#[derive(Resource, Default)]
pub struct Drawn {
    live: Option<Entity>,
    last: Option<Vec3>,
    vel: Vec3,
}

/// Marks the hull entity, for `fx/gun.rs` to find the muzzle on.
#[derive(Component)]
pub struct HeliBody;

/// A spinning rotor and its axis in the hull's frame.
#[derive(Component)]
pub struct Rotor {
    axis: Vec3,
    rad_s: f32,
}

pub fn load(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.insert_resource(HeliAssets {
        hull: meshes.add(boxes_mesh_with(HULL, linear, 1.0)),
        rotor: meshes.add(boxes_mesh_with(ROTOR, linear, 1.0)),
        tail: meshes.add(boxes_mesh_with(TAIL_ROTOR, linear, 1.0)),
        material: materials.add(StandardMaterial {
            base_color: Color::WHITE,
            perceptual_roughness: 0.6,
            metallic: 0.2,
            ..default()
        }),
    });
    commands.init_resource::<Drawn>();
}

pub fn stream(
    mut commands: Commands,
    mut drawn: ResMut<Drawn>,
    mut hulls: Query<&mut Transform, (With<HeliBody>, Without<Rotor>)>,
    mut rotors: Query<(&mut Transform, &Rotor), Without<HeliBody>>,
    time: Res<Time>,
    assets: Option<Res<HeliAssets>>,
    net: NonSend<Net>,
) {
    let Some(assets) = assets else {
        return; // Startup has not run yet.
    };
    let core = &net.session.core;
    let id = mob::mob_id(HELI_SLOT);
    let mut rs = client_core::interp::RemoteState::default();
    let present = core.interp.ids().any(|i| i == id);
    if !present {
        // Out of interest, or gone back out to sea.
        if let Some(e) = drawn.live.take() {
            commands.entity(e).despawn();
        }
        drawn.last = None;
        drawn.vel = Vec3::ZERO;
        return;
    }
    if !core.interp.sample(id, core.render_tick(), &mut rs) {
        return;
    }
    let dt = time.delta_secs();
    let pos = Vec3::new(rs.x, rs.y, rs.z);
    if let (Some(last), true) = (drawn.last, dt > 0.0) {
        let v = (pos - last) / dt;
        let k = (VEL_SMOOTH * dt).min(1.0);
        let vel = drawn.vel;
        drawn.vel = vel + (v - vel) * k;
    }
    drawn.last = Some(pos);

    // Heading off the wire, then the bank toward the horizontal velocity:
    // a tilt about the axis square to it, so the rotor disc leans into the
    // way it is going whichever way the nose points.
    let facing = Quat::from_rotation_y(rs.yaw * (std::f32::consts::TAU / 65536.0));
    let flat = Vec3::new(drawn.vel.x, 0.0, drawn.vel.z);
    let speed = flat.length();
    let bank = if speed > 0.5 {
        let axis = Vec3::Y.cross(flat / speed);
        Quat::from_axis_angle(axis, (speed * BANK_PER_MPS).min(BANK_MAX))
    } else {
        Quat::IDENTITY
    };
    let rotation = bank * facing;

    match drawn.live {
        Some(e) => {
            if let Ok(mut t) = hulls.get_mut(e) {
                t.translation = pos;
                t.rotation = rotation;
            }
        }
        None => {
            let e = commands
                .spawn((
                    super::WorldEntity,
                    HeliBody,
                    Mesh3d(assets.hull.clone()),
                    MeshMaterial3d(assets.material.clone()),
                    Transform::from_translation(pos).with_rotation(rotation),
                ))
                // Children, not `WorldEntity`: the teardown marks roots
                // and a recursive despawn takes them (`mobs.rs`'s legs).
                .with_children(|parent| {
                    parent.spawn((
                        Rotor {
                            axis: Vec3::Y,
                            rad_s: ROTOR_RAD_S,
                        },
                        Mesh3d(assets.rotor.clone()),
                        MeshMaterial3d(assets.material.clone()),
                        Transform::from_translation(ROTOR_AT),
                    ));
                    parent.spawn((
                        Rotor {
                            axis: Vec3::X,
                            rad_s: TAIL_RAD_S,
                        },
                        Mesh3d(assets.tail.clone()),
                        MeshMaterial3d(assets.material.clone()),
                        Transform::from_translation(TAIL_ROTOR_AT),
                    ));
                })
                .id();
            drawn.live = Some(e);
        }
    }

    for (mut t, rotor) in rotors.iter_mut() {
        t.rotate_local(Quat::from_axis_angle(rotor.axis, rotor.rad_s * dt));
    }
}
