//! The lore, drawn (`ARC.md` F5–F7): the speakers as people (the player
//! rig at idle, the shopkeepers' shade), the inscriptions as black slabs
//! ([`SLAB_GLB`]) with a gold ring on top, and each mechanism's dials as
//! stone drums ([`DIAL_GLB`]) whose notch turns as the dials do. Where each
//! stands is `sim_core::spot`'s answer, so it is drawn where the sim
//! measures reach from.

use bevy::prelude::*;

use super::{Net, WorldEntity, WorldId};

/// An inscription's stone: a blank basalt stele on a footing, 1.31 m wide,
/// 2.0 m tall, generated. The ring on top stays an exact torus.
pub const SLAB_GLB: &str = "models/site/lore_slab.glb";
/// How far the stele stands in the ground (`ART.md` rule 2).
const SLAB_SINK_M: f32 = 0.1;
/// A dial's drum: a basalt drum 1.1 m across, its top at 1.0 m, generated.
pub const DIAL_GLB: &str = "models/site/lore_dial.glb";

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
    server: Res<AssetServer>,
    have: Query<&LoreMark>,
) {
    let Some(world) = world else { return };
    let lore = &net.session.core.lore;
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
        let (slab, slab_material) = super::mobs::model(&server, SLAB_GLB);
        let ring = materials.add(gold());
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
                    Mesh3d(slab),
                    MeshMaterial3d(slab_material),
                    Transform::from_xyz(0.0, -SLAB_SINK_M, 0.0),
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
            let (drum, drum_material) = super::mobs::model(&server, DIAL_GLB);
            let notch = materials.add(gold());
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
                    c.spawn((Mesh3d(drum), MeshMaterial3d(drum_material)));
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
