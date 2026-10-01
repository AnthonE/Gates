//! The Black Ziggurat (`sim_core::monument`), drawn: the kit's boxes through
//! the town's builder, or the Blender dressing
//! (`assets/models/site/ziggurat.glb`) once it loads. The three card doors
//! are drawn apart from the rest so a door the server opened
//! (`ClientCore::card_doors`) can stand aside.

use bevy::prelude::*;
use sim_core::kit::KitPart;
use sim_core::monument::{self, Ziggurat};

use super::depot::{self, Surface, SURFACES};
use super::town::{kit_meshes, kit_transform};
use super::{WorldEntity, WorldId};

#[derive(Component)]
pub struct ZigguratVisual;

/// The box stand-in, despawned once the dressed model is in.
#[derive(Component)]
pub struct ZigguratFallback;

/// One card door's leaf: hidden while that door stands open.
#[derive(Component)]
pub struct DoorLeaf(pub u8);

#[derive(Resource)]
pub struct ZigguratModel {
    gltf: Handle<bevy::gltf::Gltf>,
    done: bool,
}

pub const ZIGGURAT_GLB: &str = "models/site/ziggurat.glb";

/// The lapis glow at each reader and lever. **(knob)**
const READER_COLOR: Color = Color::srgb(0.35, 0.6, 1.0);
const READER_LUMENS: f32 = 600.0;
const READER_RANGE_M: f32 = 7.0;

pub fn transform(z: &Ziggurat) -> Transform {
    kit_transform(z.x, z.z, z.floor_y, z.rot)
}

/// Every part but the door leaves.
fn body_parts() -> Vec<KitPart> {
    monument::PARTS
        .iter()
        .copied()
        .filter(|p| p.door == sim_core::kit::NO_DOOR)
        .collect()
}

/// Build the ziggurat once per world.
pub fn spawn(
    mut commands: Commands,
    world: Res<WorldId>,
    server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    drawn: Query<(), With<ZigguratVisual>>,
) {
    if !drawn.is_empty() {
        return;
    }
    let root = commands
        .spawn((
            ZigguratVisual,
            WorldEntity,
            Transform::IDENTITY,
            Visibility::default(),
        ))
        .id();
    let z = world.haven.ziggurat;
    if !z.live {
        return;
    }
    let mats: Vec<Handle<StandardMaterial>> = SURFACES
        .iter()
        .map(|&s| materials.add(depot::material(s, &server)))
        .collect();
    let tf = transform(&z);
    for (surface, mesh) in kit_meshes(&body_parts()) {
        commands.spawn((
            ChildOf(root),
            ZigguratFallback,
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(mats[surface as usize].clone()),
            tf,
        ));
    }
    // The leaves: steel slabs with a lapis stripe, one entity per door.
    for d in 0..monument::CARD_DOORS {
        let leaf: Vec<KitPart> = monument::PARTS
            .iter()
            .copied()
            .filter(|p| p.door as usize == d + 1)
            .collect();
        for (surface, mesh) in kit_meshes(&leaf) {
            commands.spawn((
                ChildOf(root),
                DoorLeaf(d as u8),
                Mesh3d(meshes.add(mesh)),
                MeshMaterial3d(mats[surface as usize].clone()),
                tf,
                Visibility::default(),
            ));
        }
        for inside in [false, true] {
            if let Some((x, y, wz)) = monument::reader_world(&z, d, inside) {
                commands.spawn((
                    ChildOf(root),
                    PointLight {
                        color: READER_COLOR,
                        intensity: READER_LUMENS,
                        range: READER_RANGE_M,
                        shadows_enabled: false,
                        ..default()
                    },
                    Transform::from_xyz(x, y + 2.2, wz),
                ));
            }
        }
    }
    commands.insert_resource(ZigguratModel {
        gltf: server.load(ZIGGURAT_GLB),
        done: false,
    });
}

/// Swap the boxes for the dressed model once it has loaded (the town's
/// `dress`, for this kit). The door leaves stay as they are.
#[allow(clippy::too_many_arguments)]
pub fn dress(
    mut commands: Commands,
    model: Option<ResMut<ZigguratModel>>,
    world: Res<WorldId>,
    server: Res<AssetServer>,
    gltfs: Res<Assets<bevy::gltf::Gltf>>,
    gmeshes: Res<Assets<bevy::gltf::GltfMesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    root: Query<Entity, With<ZigguratVisual>>,
    fallback: Query<Entity, With<ZigguratFallback>>,
) {
    let Some(mut model) = model else { return };
    if model.done {
        return;
    }
    let Some(gltf) = gltfs.get(&model.gltf) else {
        if let Some(bevy::asset::LoadState::Failed(_)) = server.get_load_state(&model.gltf) {
            model.done = true;
        }
        return;
    };
    let Ok(root) = root.single() else { return };
    let mut parts = Vec::new();
    for (name, handle) in gltf.named_meshes.iter() {
        let Some(surface) = Surface::from_role(name) else {
            continue;
        };
        let Some(gm) = gmeshes.get(handle) else {
            return;
        };
        for prim in &gm.primitives {
            parts.push((surface, prim.mesh.clone()));
        }
    }
    if parts.is_empty() {
        return;
    }
    model.done = true;
    let tf = transform(&world.haven.ziggurat);
    let mut mats: Vec<Option<Handle<StandardMaterial>>> = vec![None; SURFACES.len()];
    for (surface, mesh) in parts {
        let mat = mats[surface as usize]
            .get_or_insert_with(|| {
                let mut m = depot::material(surface, &server);
                m.uv_transform = bevy::math::Affine2::from_scale(Vec2::splat(surface.tiles()));
                materials.add(m)
            })
            .clone();
        commands.spawn((ChildOf(root), Mesh3d(mesh), MeshMaterial3d(mat), tf));
    }
    for e in fallback.iter() {
        commands.entity(e).despawn();
    }
}

/// Hide a leaf while its door stands open.
pub fn doors(net: NonSend<super::Net>, mut q: Query<(&DoorLeaf, &mut Visibility)>) {
    let open = net.session.core.card_doors;
    for (leaf, mut vis) in q.iter_mut() {
        let want = if open & (1 << leaf.0) != 0 {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        };
        if *vis != want {
            *vis = want;
        }
    }
}
