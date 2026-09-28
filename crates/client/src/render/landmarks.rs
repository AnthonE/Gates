//! The landmarks (`sim_core::landmark`), drawn: each kit's boxes through the
//! depot's own cuboid builder and photographed surfaces, so a ruin's walls are
//! masonry, a mast is weathered steel and a yard's containers are painted
//! sheet — the same boxes the sim collides against, and nothing else.
//!
//! Built once per world and drawn at any distance: a 44 m mast on a summit is
//! the point of it, and the whole set is a few thousand triangles.

use bevy::prelude::*;
use sim_core::landmark::{self, Landmark, Mat};

use super::depot::{self, Surface, SURFACES};
use super::{WorldEntity, WorldId};

#[derive(Component)]
pub struct LandmarkVisual;

fn surface(mat: Mat) -> (Surface, [f32; 3]) {
    match mat {
        Mat::Stone => (Surface::Stone, [0.92, 0.9, 0.86]),
        Mat::Concrete => (Surface::Concrete, [1.0; 3]),
        Mat::Steel => (Surface::Steel, depot::DEPOT_STEEL_TINT),
        Mat::Timber => (Surface::Timber, [1.0; 3]),
        Mat::Cargo => (Surface::Cargo, [1.0; 3]),
    }
}

/// One landmark's boxes in its own frame, grouped by surface.
pub fn landmark_meshes(m: &Landmark) -> Vec<(Surface, Mesh)> {
    let mut soups = depot::kit();
    for (i, part) in landmark::parts(m.kind).iter().enumerate() {
        let (s, mut tint) = surface(part.mat);
        if part.mat == Mat::Cargo {
            // Each container its own paint.
            tint = depot::DEPOT_CARGO_TINTS[(i + m.yaw as usize) % depot::DEPOT_CARGO_TINTS.len()];
        }
        depot::cuboid(&mut soups, part.b, s, tint);
    }
    depot::finish(soups)
}

/// The landmark's frame: `landmark::to_world`'s rotation, on its base.
pub fn transform(m: &Landmark) -> Transform {
    let (s, c) = sim_core::yaw_dir((m.yaw as u16) << 8);
    // Local +X maps to (c, -s) and local +Z to (s, c) in world XZ, which is a
    // rotation about +Y by atan2(s, c).
    let angle = s.atan2(c);
    Transform {
        translation: Vec3::new(m.x, m.y, m.z),
        rotation: Quat::from_rotation_y(angle),
        scale: Vec3::ONE,
    }
}

/// Build every landmark once per world.
pub fn spawn(
    mut commands: Commands,
    world: Res<WorldId>,
    server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    drawn: Query<(), With<LandmarkVisual>>,
) {
    if !drawn.is_empty() {
        return;
    }
    let mats: Vec<Handle<StandardMaterial>> = SURFACES
        .iter()
        .map(|&s| materials.add(depot::material(s, &server)))
        .collect();
    let root = commands
        .spawn((
            LandmarkVisual,
            WorldEntity,
            Transform::IDENTITY,
            Visibility::default(),
        ))
        .id();
    for m in world.haven.marks.iter().filter(|m| m.live) {
        let t = transform(m);
        for (surface, mesh) in landmark_meshes(m) {
            commands.spawn((
                ChildOf(root),
                Mesh3d(meshes.add(mesh)),
                MeshMaterial3d(mats[surface as usize].clone()),
                t,
            ));
        }
    }
}
