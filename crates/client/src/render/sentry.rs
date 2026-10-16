//! THE GATE's sentries — the client half (`sim-core/src/sentry.rs`).
//!
//! On the wire each is a roster slot below the heli's, a mob record whose
//! yaw is where the gun points, so it arrives through the same interpolator.
//! This file takes those ids away from `mobs::stream` and draws a turret on
//! each watchtower roof: a fixed pedestal (the player turret's own base,
//! [`TURRET_BASE_GLB`]), a generated head ([`HEAD_GLB`]) that turns to the
//! wire's yaw, and a lamp that burns green while the gun is idle and red for
//! a few seconds after it locks on to somebody (`EventMsg::SentryLock`).
//!
//! **The origin is the feet**, like every body: the sim's gun is
//! `ranged::ARROW_EYE_MM` above it, which is where the barrel sits here and
//! where `fx/gun.rs` draws the round from.

use bevy::prelude::*;

use super::props::{boxes_mesh_with, linear};
use super::structures::TURRET_BASE_GLB;
use super::Net;
use sim_core::limits::MAX_TURRETS;
use sim_core::mob;
use sim_core::sentry::{is_sentry_slot, SENTRIES, SENTRY_SLOT0};
use sim_core::turret::{is_turret_slot, TURRET_SLOT0};

/// Is this wire id one of the town's sentries, or a player's turret?
#[inline]
pub fn is_sentry(id: u32) -> bool {
    mob::slot_of_id(id).is_some_and(|s| is_sentry_slot(s) || is_turret_slot(s))
}

/// Is this wire id a player's auto turret (`sim_core::turret`)?
#[inline]
pub fn is_turret(id: u32) -> bool {
    mob::slot_of_id(id).is_some_and(is_turret_slot)
}

/// Every gun drawn here: the town's sentries, then the player turrets.
const GUNS: usize = SENTRIES + MAX_TURRETS;

/// Gun `k`'s roster slot.
const fn gun_slot(k: usize) -> usize {
    if k < SENTRIES {
        SENTRY_SLOT0 + k
    } else {
        TURRET_SLOT0 + k - SENTRIES
    }
}

/// A gun's index here from its roster slot.
fn gun_of_slot(s: usize) -> Option<usize> {
    if is_sentry_slot(s) {
        Some(s - SENTRY_SLOT0)
    } else if is_turret_slot(s) {
        Some(SENTRIES + s - TURRET_SLOT0)
    } else {
        None
    }
}

const STEEL: u32 = 0x2b2d2f;

/// The pedestal stands on the tower roof (`town::SENTRY_POSTS` puts the gun
/// 0.8 m over the roof and the feet 1.6 m under the gun, so the roof is at
/// y = 0.8 here): the player turret's base model, whose top meets the head.
const PEDESTAL_AT: Vec3 = Vec3::new(0.0, 0.8, 0.0);

/// A player turret's neck: the deployable's own base (`render/structures.rs`)
/// is its base, 0.6 m tall, and the sim's gun is `turret::GUN_M` over that
/// base — 1.6 m over the record's feet, like a sentry's — so the neck
/// bridges the top of the box to the head.
const NECK: &[([f32; 3], [f32; 3], u32)] = &[([0.0, 1.42, 0.0], [0.09, 0.13, 0.09], STEEL)];

/// The head: a generated armoured housing with one barrel, facing **+Z**
/// (the sim's yaw 0), pivoting in the middle of its housing
/// (`ci/import_meshy.py --shift`). 0.8 × 0.4 × 0.9 m before [`HEAD_SCALE`].
pub const HEAD_GLB: &str = "models/site/sentry_head.glb";
/// The head's pivot, in turret space.
const HEAD_AT: Vec3 = Vec3::new(0.0, 1.6, 0.0);
/// A town head is drawn this much bigger than true size: at true size it is a
/// bump on the roof edge from the street 35 m below. The muzzle and the lamp
/// ride the head's transform, so they scale with it.
const HEAD_SCALE: f32 = 1.6;
/// The muzzle, in head space — where the flash goes.
pub const MUZZLE: Vec3 = Vec3::new(0.0, 0.07, 0.62);
/// The status lamp, in head space: on the housing's roof.
const LAMP_AT: Vec3 = Vec3::new(-0.18, 0.21, 0.05);

/// How long the lamp stays red after a lock-on, seconds.
const ALARM_S: f32 = 6.0;
/// The lamp's glow, linear: idle green, alarm red.
const IDLE: LinearRgba = LinearRgba::rgb(0.2, 2.4, 0.4);
const ALARM: LinearRgba = LinearRgba::rgb(6.0, 0.3, 0.2);

#[derive(Resource)]
pub struct SentryAssets {
    base: Handle<Mesh>,
    base_material: Handle<StandardMaterial>,
    neck: Handle<Mesh>,
    head: Handle<Mesh>,
    head_material: Handle<StandardMaterial>,
    lamp: Handle<Mesh>,
    material: Handle<StandardMaterial>,
    idle: Handle<StandardMaterial>,
    alarm: Handle<StandardMaterial>,
}

/// The drawn turrets, by sentry index, and how long each lamp stays red.
#[derive(Resource, Default)]
pub struct Drawn {
    live: [Option<Entity>; GUNS],
    alarm: [f32; GUNS],
}

/// Marks a turret's pedestal, the root the head hangs on.
#[derive(Component)]
pub struct SentryBase;

/// Marks a turret's head, for `fx/gun.rs` to find the muzzle on.
#[derive(Component)]
pub struct SentryHead(pub u32);

/// Marks a turret's status lamp.
#[derive(Component)]
pub struct SentryLamp(pub usize);

pub fn load(
    mut commands: Commands,
    server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let lamp = |emissive: LinearRgba| StandardMaterial {
        base_color: Color::BLACK,
        emissive,
        unlit: true,
        ..default()
    };
    let (base, base_material) = super::mobs::model(&server, TURRET_BASE_GLB);
    let (head, head_material) = super::mobs::model(&server, HEAD_GLB);
    commands.insert_resource(SentryAssets {
        base,
        base_material,
        neck: meshes.add(boxes_mesh_with(NECK, linear, 1.0)),
        head,
        head_material,
        lamp: meshes.add(Cuboid::new(0.07, 0.07, 0.07)),
        material: materials.add(StandardMaterial {
            base_color: Color::WHITE,
            perceptual_roughness: 0.55,
            metallic: 0.4,
            ..default()
        }),
        idle: materials.add(lamp(IDLE)),
        alarm: materials.add(lamp(ALARM)),
    });
    commands.init_resource::<Drawn>();
}

#[allow(clippy::too_many_arguments)]
pub fn stream(
    mut commands: Commands,
    mut drawn: ResMut<Drawn>,
    mut roots: Query<&mut Transform, (With<SentryBase>, Without<SentryHead>)>,
    mut heads: Query<(&SentryHead, &mut Transform), Without<SentryLamp>>,
    mut lamps: Query<(&SentryLamp, &mut MeshMaterial3d<StandardMaterial>)>,
    feed: Res<super::feed::Feed>,
    time: Res<Time>,
    assets: Option<Res<SentryAssets>>,
    net: NonSend<Net>,
) {
    let Some(assets) = assets else {
        return; // Startup has not run yet.
    };
    let core = &net.session.core;
    let dt = time.delta_secs();
    for &(gun, _) in feed.sentry_locks() {
        if let Some(k) = mob::slot_of_id(gun).and_then(gun_of_slot) {
            drawn.alarm[k] = ALARM_S;
        }
    }
    let mut rs = client_core::interp::RemoteState::default();
    for k in 0..GUNS {
        drawn.alarm[k] = (drawn.alarm[k] - dt).max(0.0);
        let id = mob::mob_id(gun_slot(k));
        // A town sentry stands on its pedestal and draws big for the street
        // below; a player's turret stands on its own box at true size.
        let town = k < SENTRIES;
        let present = core.interp.ids().any(|i| i == id);
        if !present {
            if let Some(e) = drawn.live[k].take() {
                commands.entity(e).despawn();
            }
            continue;
        }
        if !core.interp.sample(id, core.render_tick(), &mut rs) {
            continue;
        }
        let pos = Vec3::new(rs.x, rs.y, rs.z);
        let aim = Quat::from_rotation_y(rs.yaw * (std::f32::consts::TAU / 65536.0));
        match drawn.live[k] {
            Some(e) => match roots.get_mut(e) {
                Ok(mut t) => t.translation = pos,
                // Torn down with the last world: stand a new one next frame.
                Err(_) => drawn.live[k] = None,
            },
            None => {
                let e = commands
                    .spawn((
                        super::WorldEntity,
                        SentryBase,
                        Transform::from_translation(pos),
                        Visibility::default(),
                    ))
                    // Children, not `WorldEntity`: the teardown marks roots
                    // and a recursive despawn takes them.
                    .with_children(|parent| {
                        if town {
                            parent.spawn((
                                Mesh3d(assets.base.clone()),
                                MeshMaterial3d(assets.base_material.clone()),
                                Transform::from_translation(PEDESTAL_AT),
                            ));
                        } else {
                            parent.spawn((
                                Mesh3d(assets.neck.clone()),
                                MeshMaterial3d(assets.material.clone()),
                            ));
                        }
                        parent
                            .spawn((
                                SentryHead(id),
                                Mesh3d(assets.head.clone()),
                                MeshMaterial3d(assets.head_material.clone()),
                                Transform::from_translation(HEAD_AT)
                                    .with_rotation(aim)
                                    .with_scale(Vec3::splat(if town { HEAD_SCALE } else { 1.0 })),
                            ))
                            .with_children(|head| {
                                head.spawn((
                                    SentryLamp(k),
                                    Mesh3d(assets.lamp.clone()),
                                    MeshMaterial3d(assets.idle.clone()),
                                    Transform::from_translation(LAMP_AT),
                                ));
                            });
                    })
                    .id();
                drawn.live[k] = Some(e);
            }
        }
        for (head, mut t) in heads.iter_mut() {
            if head.0 == id {
                t.rotation = aim;
            }
        }
    }
    for (lamp, mut mat) in lamps.iter_mut() {
        let want = if drawn.alarm[lamp.0] > 0.0 {
            &assets.alarm
        } else {
            &assets.idle
        };
        if mat.0 != *want {
            mat.0 = want.clone();
        }
    }
}
