//! The town ("THE GATE", `sim_core::town`), drawn: the kit's boxes through
//! the depot's cuboid builder and photographed surfaces — the same boxes the
//! sim collides against, so there is never an invisible wall. When the
//! Blender dressing (`assets/models/site/town.glb`, `ci/site_kit.py`) is
//! present it replaces the boxes with the dressed meshes.
//!
//! Built once per world and drawn at any distance: the pylons are the point.

use bevy::prelude::*;
use sim_core::kit::{KitMat, KitPart};
use sim_core::town::{self, Town};

use super::depot::{self, Surface, SURFACES};
use super::weathering::{self, Dressed, MonumentMaterial};
use super::{WorldEntity, WorldId};

#[derive(Component)]
pub struct TownVisual;

/// The cuboid stand-in, despawned once the dressed model is in.
#[derive(Component)]
pub struct TownFallback;

/// The dressed model: loading, then swapped in once (`dress`).
#[derive(Resource)]
pub struct TownModel {
    gltf: Handle<bevy::gltf::Gltf>,
    done: bool,
}

impl TownModel {
    /// Dressed, or given up on — what the loading screen waits for, so the
    /// boxes never swap for the dressed town in front of a player.
    pub fn ready(&self) -> bool {
        self.done
    }
}

/// The Blender dressing (`ci/site_kit.py gen --kit ci/kits/town.json`).
pub const TOWN_GLB: &str = "models/site/town.glb";

/// A kit material as one of the photographed surfaces and a tint.
pub fn surface(mat: KitMat, index: usize) -> (Surface, [f32; 3]) {
    match mat {
        KitMat::Yard => (Surface::Yard, depot::DEPOT_YARD_TINT),
        KitMat::Concrete => (Surface::Concrete, [1.0; 3]),
        KitMat::Sheet => (Surface::Sheet, depot::DEPOT_SHEET_TINT),
        KitMat::Cargo => (
            Surface::Cargo,
            depot::DEPOT_CARGO_TINTS[index % depot::DEPOT_CARGO_TINTS.len()],
        ),
        KitMat::Timber => (Surface::Timber, [1.0; 3]),
        KitMat::Steel => (Surface::Steel, depot::DEPOT_STEEL_TINT),
        KitMat::Obsidian => (Surface::Obsidian, [1.0; 3]),
        KitMat::Gilt => (Surface::Gilt, [1.0; 3]),
        KitMat::Canvas => (Surface::Canvas, [0.78, 0.66, 0.48]),
        KitMat::Lapis => (Surface::Lapis, [1.0; 3]),
    }
}

/// A surface as a Blender dressing wears it (`ci/site_kit.py`): the
/// photographed maps at the surface's tile over the model's metre UVs, and
/// both sides lit — sheets, awnings and trim are single faces seen from
/// both sides, and a back face lights with its normal flipped.
pub fn dressed_material(surface: Surface, server: &AssetServer) -> StandardMaterial {
    let mut m = depot::material(surface, server);
    m.uv_transform = bevy::math::Affine2::from_scale(Vec2::splat(surface.tiles()));
    m.double_sided = true;
    m.cull_mode = None;
    m
}

/// A kit's boxes in its own frame, grouped by surface.
pub fn kit_meshes(parts: &[KitPart]) -> Vec<(Surface, Mesh)> {
    let mut soups = depot::kit();
    for (i, part) in parts.iter().enumerate() {
        let (s, tint) = surface(part.mat, i);
        depot::cuboid(&mut soups, part.b, s, tint);
    }
    depot::finish(soups)
}

/// The kit's frame in the world: its floor, turned by quarter turns.
pub fn kit_transform(x: f32, z: f32, floor_y: f32, rot: u8) -> Transform {
    Transform {
        translation: Vec3::new(x, floor_y, z),
        rotation: Quat::from_rotation_y((rot & 3) as f32 * std::f32::consts::FRAC_PI_2),
        scale: Vec3::ONE,
    }
}

pub fn transform(t: &Town) -> Transform {
    kit_transform(t.x, t.z, t.floor_y, t.rot)
}

/// Build the town once per world.
pub fn spawn(
    mut commands: Commands,
    world: Res<WorldId>,
    server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    drawn: Query<(), With<TownVisual>>,
) {
    if !drawn.is_empty() {
        return;
    }
    let root = commands
        .spawn((
            TownVisual,
            WorldEntity,
            Transform::IDENTITY,
            Visibility::default(),
        ))
        .id();
    let t = world.haven.town;
    if !t.live {
        return;
    }
    let mats: Vec<Handle<StandardMaterial>> = SURFACES
        .iter()
        .map(|&s| materials.add(depot::material(s, &server)))
        .collect();
    let mut glow = TownGlow::default();
    glow.track(Surface::Bulb, &mats[Surface::Bulb as usize]);
    glow.track(Surface::Lapis, &mats[Surface::Lapis as usize]);
    let tf = transform(&t);
    for (surface, mesh) in kit_meshes(town::PARTS) {
        commands.spawn((
            ChildOf(root),
            TownFallback,
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(mats[surface as usize].clone()),
            tf,
        ));
    }
    for (lx, ly, lz) in town::LAMPS {
        let (wx, wz) = sim_core::kit::to_world(&t.placed(), lx, lz);
        commands.spawn((
            ChildOf(root),
            TownLamp,
            PointLight {
                color: LAMP_COLOR,
                intensity: 0.0,
                range: LAMP_RANGE_M,
                shadows_enabled: false,
                ..default()
            },
            Transform::from_xyz(wx, t.floor_y + ly, wz),
        ));
    }
    commands.insert_resource(glow);
    commands.insert_resource(TownModel {
        gltf: server.load(TOWN_GLB),
        done: false,
    });
}

/// Swap the cuboids for the dressed model once it has loaded. One mesh per
/// surface role, each on that surface's photographed material with the
/// model's metre UVs scaled to the surface's tile.
#[allow(clippy::too_many_arguments)]
pub fn dress(
    mut commands: Commands,
    model: Option<ResMut<TownModel>>,
    world: Res<WorldId>,
    server: Res<AssetServer>,
    gltfs: Res<Assets<bevy::gltf::Gltf>>,
    gmeshes: Res<Assets<bevy::gltf::GltfMesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut weathered: ResMut<Assets<MonumentMaterial>>,
    mut glow: Option<ResMut<TownGlow>>,
    root: Query<Entity, With<TownVisual>>,
    fallback: Query<Entity, With<TownFallback>>,
) {
    let Some(mut model) = model else { return };
    if model.done {
        return;
    }
    let Some(gltf) = gltfs.get(&model.gltf) else {
        if let Some(bevy::asset::LoadState::Failed(e)) = server.get_load_state(&model.gltf) {
            warn!("town: {TOWN_GLB} did not load ({e}); keeping the boxes");
            model.done = true;
        }
        return;
    };
    let Ok(root) = root.single() else { return };
    let mut parts = Vec::new();
    for (name, handle) in gltf.named_meshes.iter() {
        let Some(surface) = Surface::from_role(name) else {
            warn!("town: {TOWN_GLB} mesh {name:?} names no surface; skipped");
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
    let tf = transform(&world.haven.town);
    let mut mats: Vec<Option<Dressed>> = vec![None; SURFACES.len()];
    for (surface, mesh) in parts {
        let mat = mats[surface as usize].get_or_insert_with(|| {
            let d = weathering::dress(
                surface,
                dressed_material(surface, &server),
                &mut materials,
                &mut weathered,
            );
            if let (Some(glow), Some(h)) = (glow.as_mut(), d.plain()) {
                glow.track(surface, h);
            }
            d
        });
        mat.insert(&mut commands.spawn((ChildOf(root), Mesh3d(mesh), tf)));
    }
    let mut gone = 0;
    for e in fallback.iter() {
        commands.entity(e).despawn();
        gone += 1;
    }
    info!("town: dressed, {gone} stand-in meshes removed");
}

/// A town lamp's light, dark by day.
#[derive(Component)]
pub struct TownLamp;

/// Sodium-ish warm white. **(knob)**
const LAMP_COLOR: Color = Color::srgb(1.0, 0.78, 0.5);
/// Lumens at night — a street lamp, brighter than a fire. **(knob)**
const LAMP_LUMENS: f32 = 2400.0;
const LAMP_RANGE_M: f32 = 16.0;

/// The town's lit materials — the bulbs and the lapis seams — so the
/// night can turn them up: a bulb is dark at noon, and the pylons' seams
/// burn brighter after dusk, a beacon from across the island (Rust's
/// Bandit Town got lights visible from long range for the same reason).
#[derive(Resource, Default)]
pub struct TownGlow {
    bulbs: Vec<Handle<StandardMaterial>>,
    lapis: Vec<Handle<StandardMaterial>>,
    /// The night weight last written, so a still dusk costs nothing.
    at: Option<f32>,
}

impl TownGlow {
    fn track(&mut self, surface: Surface, h: &Handle<StandardMaterial>) {
        match surface {
            Surface::Bulb => self.bulbs.push(h.clone()),
            Surface::Lapis => self.lapis.push(h.clone()),
            _ => return,
        }
        self.at = None;
    }
}

/// How dark it has to be for the lamps to be full on: by the sun's
/// elevation, radians, fading in over [`LIT_SPAN`] around the horizon so
/// dusk brings them up rather than a switch. **(knob)**
const LIT_BELOW: f32 = 0.10;
const LIT_SPAN: f32 = 0.22;
/// The bulbs' glow at full night (`depot::material`'s value), linear.
const BULB_GLOW: LinearRgba = LinearRgba::rgb(6.0, 4.5, 2.2);
/// The lapis seams by day and at full night, linear. **(knob)**
const LAPIS_DAY: LinearRgba = LinearRgba::rgb(0.25, 0.55, 1.6);
const LAPIS_NIGHT: LinearRgba = LinearRgba::rgb(1.0, 2.3, 6.5);

/// How much of the night it is, 0 at day to 1 at full dark, off the sun.
pub fn night_weight(day_frac: f32) -> f32 {
    ((LIT_BELOW - super::rig::sun_elevation(day_frac)) / LIT_SPAN).clamp(0.0, 1.0)
}

/// Lamps, bulbs and seams follow the dark (the server's clock, as the birds
/// read it), fading through dusk and dawn.
pub fn lamps(
    feed: Res<super::feed::Feed>,
    pin: Res<super::rig::DayPin>,
    glow: Option<ResMut<TownGlow>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut q: Query<&mut PointLight, With<TownLamp>>,
) {
    let Some(mut glow) = glow else { return };
    let tick = pin.day_tick(feed.server_tick_est, &feed.env);
    let w = night_weight(sim_core::world::day_frac(tick));
    if glow.at.is_some_and(|a| (a - w).abs() < 0.01) {
        return;
    }
    glow.at = Some(w);
    for mut l in q.iter_mut() {
        l.intensity = LAMP_LUMENS * w;
    }
    for h in &glow.bulbs {
        if let Some(m) = materials.get_mut(h) {
            m.emissive = BULB_GLOW * w;
        }
    }
    for h in &glow.lapis {
        if let Some(m) = materials.get_mut(h) {
            m.emissive = LAPIS_DAY * (1.0 - w) + LAPIS_NIGHT * w;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The drawn frame is the sim's: a local point lands where
    /// `kit::to_world` puts it, at every quarter turn.
    #[test]
    fn the_transform_agrees_with_the_sim_frame() {
        for rot in 0..4u8 {
            let p = sim_core::kit::Placed {
                x: 100.0,
                z: 200.0,
                floor_y: 5.0,
                rot,
            };
            let tf = kit_transform(p.x, p.z, p.floor_y, rot);
            for (lx, lz) in [(3.0, 0.0), (0.0, 7.0), (-2.0, 5.0)] {
                let w = tf.transform_point(Vec3::new(lx, 1.0, lz));
                let (sx, sz) = sim_core::kit::to_world(&p, lx, lz);
                assert!(
                    (w.x - sx).abs() < 1e-4 && (w.z - sz).abs() < 1e-4,
                    "rot {rot}"
                );
                assert!((w.y - 6.0).abs() < 1e-4);
            }
        }
    }
}
