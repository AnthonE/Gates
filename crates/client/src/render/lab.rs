//! The lab under the island (`sim_core::lab`), drawn: its boxes through the
//! town's builder in its host's frame, the same boxes the sim collides
//! against, with the door drawn apart so it stands aside while the server's
//! door mask has the lab's bit (`ClientCore::card_doors`), and lamps in
//! its rooms and down its stair.

use bevy::prelude::*;
use sim_core::kit::KitPart;
use sim_core::lab;

use super::depot::{self, SURFACES};
use super::landmarks;
use super::town::kit_meshes;
use super::{WorldEntity, WorldId};

#[derive(Component)]
pub struct LabVisual;

/// The lab's door leaf: hidden while the door stands open.
#[derive(Component)]
pub struct LabDoor;

/// The rooms' lamps: a cold work light. **(knob)**
const LAMP_COLOR: Color = Color::srgb(0.75, 0.85, 1.0);
const LAMP_LUMENS: f32 = 2500.0;
const LAMP_RANGE_M: f32 = 12.0;

/// Build the lab once per world, on an island that has one.
pub fn spawn(
    mut commands: Commands,
    world: Res<WorldId>,
    server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    drawn: Query<(), With<LabVisual>>,
) {
    if !drawn.is_empty() {
        return;
    }
    let root = commands
        .spawn((
            LabVisual,
            WorldEntity,
            Transform::IDENTITY,
            Visibility::default(),
        ))
        .id();
    let Some(m) = lab::host(&world.haven.marks).copied() else {
        return;
    };
    let mats: Vec<Handle<StandardMaterial>> = SURFACES
        .iter()
        .map(|&s| materials.add(depot::material(s, &server)))
        .collect();
    let tf = landmarks::transform(&m);
    let (body, door): (Vec<KitPart>, Vec<KitPart>) = lab::PARTS
        .iter()
        .copied()
        .partition(|p| p.door != lab::LAB_DOOR);
    for (surface, mesh) in kit_meshes(&body) {
        commands.spawn((
            ChildOf(root),
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(mats[surface as usize].clone()),
            tf,
        ));
    }
    for (surface, mesh) in kit_meshes(&door) {
        commands.spawn((
            ChildOf(root),
            LabDoor,
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(mats[surface as usize].clone()),
            tf,
            Visibility::default(),
        ));
    }
    for &(x, y, z) in lab::LIGHTS.iter() {
        commands.spawn((
            ChildOf(root),
            PointLight {
                color: LAMP_COLOR,
                intensity: LAMP_LUMENS,
                range: LAMP_RANGE_M,
                shadows_enabled: false,
                ..default()
            },
            Transform::from_translation(tf.transform_point(Vec3::new(x, y, z))),
        ));
    }
}

/// Hide the leaf while the lab's door stands open.
pub fn doors(net: NonSend<super::Net>, mut q: Query<&mut Visibility, With<LabDoor>>) {
    let open = net.session.core.card_doors & lab::LAB_DOOR_BIT != 0;
    let want = if open {
        Visibility::Hidden
    } else {
        Visibility::Inherited
    };
    for mut vis in q.iter_mut() {
        if *vis != want {
            *vis = want;
        }
    }
}
