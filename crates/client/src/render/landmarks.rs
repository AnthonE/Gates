//! The landmarks (`sim_core::landmark`), drawn: each kit's boxes through the
//! depot's own cuboid builder and photographed surfaces, so a ruin's walls are
//! masonry, a mast is weathered steel and a yard's containers are painted
//! sheet — the same boxes the sim collides against, and nothing else.
//!
//! **Rock parts are the exception** (`Mat::Rock`: the anvils, arches and
//! spires). They are drawn as fractured rocks by the boulders' own builder
//! (`boulders::rock_block`), on the ground's material, so a rock landmark is
//! the same granite as every other rock on the island. That material exists
//! only once the terrain has streamed, so the rocks are spawned on their own,
//! when it does.
//!
//! **The built kinds are dressed in Blender** (`landmark::DRESSED`:
//! `ci/site_kit.py gen --kit ci/kits/mark_<slug>.json`): coursed masonry, a
//! lattice mast, a log cabin, ribbed containers. The boxes draw until a
//! kind's `assets/models/site/mark_<slug>.glb` loads and stay if it never
//! does — the town's arrangement, one model per kind shared by every
//! landmark of it.
//!
//! Built once per world and drawn at any distance: a 44 m mast on a summit is
//! the point of it.

use bevy::prelude::*;
use sim_core::landmark::{self, Landmark, LandmarkKind, Mat};
use sim_core::terrain;

use super::boulders::{self, RockShape, RockSoup};
use super::depot::{self, Surface, SURFACES};
use super::terrain_mesh::Ring;
use super::town::dressed_material;
use super::weathering::{self, Dressed, MonumentMaterial};
use super::{WorldEntity, WorldId};

#[derive(Component)]
pub struct LandmarkVisual;

/// The rock half of the landmarks, spawned apart (see the module note).
#[derive(Component)]
pub struct LandmarkRocks;

/// A dressed kind's box stand-in, despawned once its model is in.
#[derive(Component)]
pub struct LandmarkFallback(pub LandmarkKind);

/// A landmark floodlight's lamp, dark by day (`town::lamps`' rule).
#[derive(Component)]
pub struct LandmarkLamp;

/// A floodlight throws more than a street lamp, and further. **(knob)**
const FLOOD_LUMENS: f32 = 7000.0;
const FLOOD_RANGE_M: f32 = 26.0;

/// The landmarks' bulbs, so the night can turn them up and the day down.
#[derive(Resource, Default)]
pub struct LandmarkGlow {
    bulbs: Vec<Handle<StandardMaterial>>,
    /// The night weight last written, so a still dusk costs nothing.
    at: Option<f32>,
}

/// The lamp heads of a landmark's floodlight poles, in its own frame: a
/// thin steel post standing from the ground to a lamp's height.
pub fn lamp_heads(kind: LandmarkKind) -> impl Iterator<Item = Vec3> {
    landmark::parts(kind).iter().filter_map(|p| {
        let b = p.b;
        let (dx, dy, dz) = (b[3] - b[0], b[4] - b[1], b[5] - b[2]);
        (p.mat == Mat::Steel
            && dx <= 0.35
            && dz <= 0.35
            && (5.0..10.0).contains(&dy)
            && b[1] <= 0.5)
            .then(|| Vec3::new((b[0] + b[3]) * 0.5, b[4] - 0.25, (b[2] + b[5]) * 0.5))
    })
}

/// The dressings still loading, and the surfaces they wear (one material
/// per surface, shared by every kind).
#[derive(Resource)]
pub struct LandmarkModels {
    pending: Vec<(LandmarkKind, Handle<bevy::gltf::Gltf>)>,
    mats: Vec<Option<Dressed>>,
}

/// A dressed kind's model, under `assets/`.
pub fn model_path(kind: LandmarkKind) -> String {
    format!("models/site/mark_{}.glb", landmark::slug(kind))
}

fn surface(mat: Mat) -> (Surface, [f32; 3]) {
    match mat {
        Mat::Stone | Mat::Rock => (Surface::Stone, [0.92, 0.9, 0.86]),
        Mat::Concrete => (Surface::Concrete, [1.0; 3]),
        Mat::Steel => (Surface::Steel, depot::DEPOT_STEEL_TINT),
        Mat::Timber => (Surface::Timber, [1.0; 3]),
        Mat::Cargo => (Surface::Cargo, [1.0; 3]),
        Mat::Gravel => (Surface::Yard, depot::DEPOT_YARD_TINT),
    }
}

/// One landmark's built boxes in its own frame, grouped by surface. Rock
/// parts are not among them.
pub fn landmark_meshes(m: &Landmark) -> Vec<(Surface, Mesh)> {
    let mut soups = depot::kit();
    for (i, part) in landmark::parts(m.kind).iter().enumerate() {
        if part.mat == Mat::Rock {
            continue;
        }
        let (s, mut tint) = surface(part.mat);
        if part.mat == Mat::Cargo {
            // Each container its own paint.
            tint = depot::DEPOT_CARGO_TINTS[(i + m.yaw as usize) % depot::DEPOT_CARGO_TINTS.len()];
        }
        depot::cuboid(&mut soups, part.b, s, tint);
    }
    depot::finish(soups)
}

/// One landmark's rock parts, in world space.
pub fn landmark_rocks(seed: u64, m: &Landmark, soup: &mut RockSoup) {
    let dir = sim_core::yaw_dir((m.yaw as u16) << 8);
    let moss = if m.y < terrain::TREELINE_H { 1.0 } else { 0.0 };
    for (i, part) in landmark::parts(m.kind).iter().enumerate() {
        if part.mat != Mat::Rock {
            continue;
        }
        let b = part.b;
        let (x, z) = landmark::to_world(m, (b[0] + b[3]) * 0.5, (b[2] + b[5]) * 0.5);
        boulders::rock_block(
            soup,
            &RockShape {
                x,
                z,
                hx: (b[3] - b[0]) * 0.5,
                hz: (b[5] - b[2]) * 0.5,
                dir,
                // A piece standing on another is drawn down into it.
                y0: m.y + b[1]
                    - if b[1] > -landmark::FOOTING_M {
                        boulders::STACK_SINK_M
                    } else {
                        0.0
                    },
                y1: m.y + b[4],
                tx: 0.0,
                tz: 0.0,
                key: (seed as u32) ^ (i as u32 + 1).wrapping_mul(0x9E37_79B9) ^ m.yaw as u32,
                moss,
                chisel: 0.0,
                min_seg: 2,
                value: boulders::ROCK_VALUE,
            },
        );
    }
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
#[allow(clippy::too_many_arguments)]
pub fn spawn(
    mut commands: Commands,
    world: Res<WorldId>,
    server: Res<AssetServer>,
    ground: Res<Ring>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    drawn: Query<(), With<LandmarkVisual>>,
    rocks: Query<(), With<LandmarkRocks>>,
) {
    if drawn.is_empty() {
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
        let mut pending: Vec<(LandmarkKind, Handle<bevy::gltf::Gltf>)> = Vec::new();
        for m in world.haven.marks.iter().filter(|m| m.live) {
            let t = transform(m);
            let dressed = landmark::DRESSED.contains(&m.kind);
            for (surface, mesh) in landmark_meshes(m) {
                let mut e = commands.spawn((
                    ChildOf(root),
                    Mesh3d(meshes.add(mesh)),
                    MeshMaterial3d(mats[surface as usize].clone()),
                    t,
                ));
                if dressed {
                    e.insert(LandmarkFallback(m.kind));
                }
            }
            if dressed && !pending.iter().any(|(k, _)| *k == m.kind) {
                pending.push((m.kind, server.load(model_path(m.kind))));
            }
            for head in lamp_heads(m.kind) {
                commands.spawn((
                    ChildOf(root),
                    LandmarkLamp,
                    PointLight {
                        color: super::town::LAMP_COLOR,
                        intensity: 0.0,
                        range: FLOOD_RANGE_M,
                        shadows_enabled: false,
                        ..default()
                    },
                    Transform::from_translation(t.transform_point(head)),
                ));
            }
        }
        commands.insert_resource(LandmarkGlow::default());
        commands.insert_resource(LandmarkModels {
            pending,
            mats: vec![None; SURFACES.len()],
        });
    }
    if rocks.is_empty() {
        let Some(material) = ground.ground_material() else {
            return;
        };
        let mut soup = RockSoup::default();
        for m in world.haven.marks.iter().filter(|m| m.live) {
            landmark_rocks(world.seed, m, &mut soup);
        }
        if soup.is_empty() {
            // Nothing to draw, but mark it done so this is not asked again.
            commands.spawn((LandmarkRocks, WorldEntity, Transform::IDENTITY));
            return;
        }
        commands.spawn((
            LandmarkRocks,
            WorldEntity,
            Mesh3d(meshes.add(soup.mesh())),
            MeshMaterial3d(material),
            Transform::IDENTITY,
        ));
    }
}

/// Swap each dressed kind's boxes for its model once it has loaded: the
/// model's meshes on every landmark of that kind, then the boxes gone. A
/// model that fails to load leaves its boxes standing.
#[allow(clippy::too_many_arguments)]
pub fn dress(
    mut commands: Commands,
    models: Option<ResMut<LandmarkModels>>,
    world: Res<WorldId>,
    server: Res<AssetServer>,
    gltfs: Res<Assets<bevy::gltf::Gltf>>,
    gmeshes: Res<Assets<bevy::gltf::GltfMesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut weathered: ResMut<Assets<MonumentMaterial>>,
    mut glow: Option<ResMut<LandmarkGlow>>,
    root: Query<Entity, With<LandmarkVisual>>,
    fallback: Query<(Entity, &LandmarkFallback)>,
) {
    let Some(mut models) = models else { return };
    if models.pending.is_empty() {
        return;
    }
    let Ok(root) = root.single() else { return };
    let LandmarkModels { pending, mats } = &mut *models;
    pending.retain(|(kind, handle)| {
        let Some(gltf) = gltfs.get(handle) else {
            if let Some(bevy::asset::LoadState::Failed(e)) = server.get_load_state(handle) {
                warn!(
                    "landmarks: {} did not load ({e}); keeping the boxes",
                    model_path(*kind)
                );
                return false;
            }
            return true;
        };
        let mut parts = Vec::new();
        for (name, h) in gltf.named_meshes.iter() {
            let Some(surface) = Surface::from_role(name) else {
                warn!(
                    "landmarks: {} mesh {name:?} names no surface; skipped",
                    model_path(*kind)
                );
                continue;
            };
            let Some(gm) = gmeshes.get(h) else {
                return true;
            };
            for prim in &gm.primitives {
                parts.push((surface, prim.mesh.clone()));
            }
        }
        if parts.is_empty() {
            return true;
        }
        for m in world
            .haven
            .marks
            .iter()
            .filter(|m| m.live && m.kind == *kind)
        {
            let t = transform(m);
            for (surface, mesh) in &parts {
                let mat = mats[*surface as usize].get_or_insert_with(|| {
                    let d = weathering::dress(
                        *surface,
                        dressed_material(*surface, &server),
                        &mut materials,
                        &mut weathered,
                    );
                    if let (Surface::Bulb, Some(h), Some(g)) = (*surface, d.plain(), glow.as_mut())
                    {
                        g.bulbs.push(h.clone());
                        g.at = None;
                    }
                    d
                });
                mat.insert(&mut commands.spawn((ChildOf(root), Mesh3d(mesh.clone()), t)));
            }
        }
        for (e, f) in fallback.iter() {
            if f.0 == *kind {
                commands.entity(e).despawn();
            }
        }
        info!("landmarks: {} dressed", model_path(*kind));
        false
    });
}

/// Floodlights and bulbs follow the dark, as the town's lamps do, opened by
/// the night eye's gain as a flame is.
pub fn lamps(
    feed: Res<super::feed::Feed>,
    pin: Res<super::rig::DayPin>,
    gain: Res<super::rig::FlameGain>,
    glow: Option<ResMut<LandmarkGlow>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut q: Query<&mut PointLight, With<LandmarkLamp>>,
) {
    let Some(mut glow) = glow else { return };
    let tick = pin.day_tick(feed.server_tick_est, &feed.env);
    let w = super::town::night_weight(sim_core::world::day_frac(tick)) * gain.0;
    if glow.at.is_some_and(|a| (a - w).abs() < 0.01 * w.max(1.0)) {
        return;
    }
    glow.at = Some(w);
    for mut l in q.iter_mut() {
        l.intensity = FLOOD_LUMENS * w;
    }
    for h in &glow.bulbs {
        if let Some(m) = materials.get_mut(h) {
            m.emissive = super::town::BULB_GLOW * w;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The relay and the quarry each light their floodlights; nothing else
    /// a landmark stands up is taken for a lamp post.
    #[test]
    fn floodlight_heads_are_found() {
        assert_eq!(lamp_heads(LandmarkKind::Relay).count(), 2);
        assert_eq!(lamp_heads(LandmarkKind::Quarry).count(), 1);
        assert_eq!(lamp_heads(LandmarkKind::Mast).count(), 0);
        assert_eq!(lamp_heads(LandmarkKind::Yard).count(), 0);
    }
}
