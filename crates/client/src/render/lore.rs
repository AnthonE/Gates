//! The lore, drawn (`ARC.md` F5–F7): the speakers as people (the player
//! rig at idle, the shopkeepers' shade), the inscriptions as black slabs
//! with a gold ring on top, and each mechanism's dials as stone drums whose
//! notch turns as the dials do. Where each stands is `sim_core::spot`'s
//! answer, so it is drawn where the sim measures reach from.

use bevy::prelude::*;

use super::{Net, WorldEntity, WorldId};

/// A drawn speaker, inscription or dial; what it is and its index.
#[derive(Component)]
pub struct LoreMark {
    pub kind: u8,
    pub index: u16,
}

/// A dial's notch: mechanism, dial.
#[derive(Component)]
pub struct DialNotch(pub u8, pub u8);

fn drawn(have: &Query<&LoreMark>, kind: u8, index: u16) -> bool {
    have.iter().any(|m| m.kind == kind && m.index == index)
}

/// Feet height and position of a spot on the island.
fn at(world: &WorldId, spot: &sim_core::spot::Spot) -> Option<Vec3> {
    let (x, _, z) = sim_core::spot::world(&world.haven, spot)?;
    Some(Vec3::new(
        x,
        sim_core::terrain::ground(world.seed, &world.haven, x, z),
        z,
    ))
}

/// Stand up every speaker, stone and dial whose place has arrived.
#[allow(clippy::too_many_arguments)]
pub fn spawn(
    mut commands: Commands,
    net: NonSend<Net>,
    world: Option<Res<WorldId>>,
    rig: Res<super::anim::Rig>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    have: Query<&LoreMark>,
) {
    let Some(world) = world else { return };
    let lore = &net.session.core.lore;
    let stone = || StandardMaterial {
        base_color: Color::srgb(0.07, 0.07, 0.08),
        perceptual_roughness: 0.4,
        ..default()
    };
    let gold = || StandardMaterial {
        base_color: Color::srgb(0.83, 0.64, 0.25),
        metallic: 1.0,
        perceptual_roughness: 0.3,
        ..default()
    };
    // People, once the rig has loaded (the shopkeepers' rule).
    if rig.ready() && rig.shaded() {
        if let Some(scene) = rig.scene.clone() {
            for (k, s) in lore.known_speakers() {
                if drawn(&have, protocol::ARC_SPEAKER, k as u16) {
                    continue;
                }
                let Some(p) = at(&world, &s.spot) else {
                    continue;
                };
                // Facing the middle of the site they stand at.
                let mid = match sim_core::spot::world(
                    &world.haven,
                    &sim_core::spot::Spot {
                        x_cm: 0,
                        y_cm: 0,
                        z_cm: 0,
                        ..s.spot
                    },
                ) {
                    Some((x, _, z)) => Vec3::new(x, p.y, z),
                    None => p + Vec3::Z,
                };
                let d = mid - p;
                let yaw = if d.length_squared() > 1e-4 {
                    d.x.atan2(d.z)
                } else {
                    0.0
                };
                commands.spawn((
                    WorldEntity,
                    LoreMark {
                        kind: protocol::ARC_SPEAKER,
                        index: k as u16,
                    },
                    super::anim::BodyAnim {
                        clip: Some(super::anim::Clip::Idle),
                        ..default()
                    },
                    super::anim::Reshade(super::anim::Shade::Keeper),
                    SceneRoot(scene.clone()),
                    Transform::from_translation(p)
                        .with_rotation(Quat::from_rotation_y(yaw))
                        .with_scale(Vec3::splat(rig.scale)),
                ));
            }
        }
    }
    for (k, ins) in lore.known_inscriptions() {
        if drawn(&have, protocol::ARC_INSCRIPTION, k as u16) {
            continue;
        }
        let Some(p) = at(&world, &ins.spot) else {
            continue;
        };
        let (slab, ring) = (materials.add(stone()), materials.add(gold()));
        commands
            .spawn((
                WorldEntity,
                LoreMark {
                    kind: protocol::ARC_INSCRIPTION,
                    index: k as u16,
                },
                Transform::from_translation(p),
                Visibility::default(),
            ))
            .with_children(|c| {
                c.spawn((
                    Mesh3d(meshes.add(Cuboid::new(1.3, 2.0, 0.35))),
                    MeshMaterial3d(slab),
                    Transform::from_xyz(0.0, 0.9, 0.0),
                ));
                c.spawn((
                    Mesh3d(meshes.add(Torus::new(0.28, 0.36))),
                    MeshMaterial3d(ring),
                    Transform::from_xyz(0.0, 2.15, 0.0)
                        .with_rotation(Quat::from_rotation_x(core::f32::consts::FRAC_PI_2)),
                ));
            });
    }
    for (k, m) in lore.known_mechs() {
        let def = sim_core::mech::MechDef {
            spot: m.spot,
            dials: m.n_dials,
            ..Default::default()
        };
        for d in 0..m.n_dials as usize {
            let index = (k as u16) << 8 | d as u16;
            if drawn(&have, protocol::ARC_MECH, index) {
                continue;
            }
            let Some(p) = at(&world, &sim_core::mech::dial_spot(&def, d)) else {
                continue;
            };
            let (drum, notch) = (materials.add(stone()), materials.add(gold()));
            commands
                .spawn((
                    WorldEntity,
                    LoreMark {
                        kind: protocol::ARC_MECH,
                        index,
                    },
                    Transform::from_translation(p),
                    Visibility::default(),
                ))
                .with_children(|c| {
                    c.spawn((
                        Mesh3d(meshes.add(Cylinder::new(0.42, 1.0))),
                        MeshMaterial3d(drum),
                        Transform::from_xyz(0.0, 0.5, 0.0),
                    ));
                    c.spawn((
                        DialNotch(k as u8, d as u8),
                        Transform::from_xyz(0.0, 1.0, 0.0),
                        Visibility::default(),
                    ))
                    .with_children(|n| {
                        n.spawn((
                            Mesh3d(meshes.add(Cuboid::new(0.08, 0.06, 0.36))),
                            MeshMaterial3d(notch),
                            Transform::from_xyz(0.0, 0.03, 0.2),
                        ));
                    });
                });
        }
    }
}

/// Turn each dial's notch to the notch it shows.
pub fn dials(net: NonSend<Net>, mut q: Query<(&DialNotch, &mut Transform)>) {
    let lore = &net.session.core.lore;
    for (n, mut tf) in q.iter_mut() {
        let Some(m) = lore.mechs.get(n.0 as usize) else {
            continue;
        };
        let values = m.values.max(1) as f32;
        let v = m.dials.get(n.1 as usize).copied().unwrap_or(0) as f32;
        let want = Quat::from_rotation_y(-v / values * core::f32::consts::TAU);
        if tf.rotation.angle_between(want) > 1e-3 {
            tf.rotation = want;
        }
    }
}
