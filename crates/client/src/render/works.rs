//! The works' terminals, drawn (`ARC.md` F1): a black plinth with a fire
//! bowl on it and a gold ring standing behind (the ring is `WORLD.md`
//! §11.3's shape, planted). Where it stands is `sim_core::spot`'s answer on
//! the seed, so it is drawn where the sim measures reach from.
//!
//! The work's state lights it:
//! - **sealed** — dark;
//! - **open** — a faint lapis glow, so a player can see there is something
//!   to bring things to;
//! - **burning** — fire in the bowl;
//! - **embers** — a low red light.

use bevy::prelude::*;
use sim_core::works::{WORK_LIT, WORK_OPEN};

use super::fx::world::{FireFx, Tongues};
use super::structures::FIRE_COLOR;
use super::{Net, WorldEntity, WorldId};

/// One work's terminal; the index is the work's.
#[derive(Component)]
pub struct WorkMark(pub u8);
/// The bowl's fire (light + flames), left at zero until it burns.
#[derive(Component)]
pub struct WorkFire(pub u8);
/// The glow an open work wears, and an ember's.
#[derive(Component)]
pub struct WorkGlow(pub u8);

const FIRE_LUMENS: f32 = 90_000.0;
const FIRE_RANGE_M: f32 = 14.0;
const GLOW_OPEN: Color = Color::srgb(0.35, 0.6, 1.0);
const GLOW_EMBER: Color = Color::srgb(0.9, 0.25, 0.1);
const GLOW_LUMENS: f32 = 2_500.0;
const PLINTH_H: f32 = 1.0;

/// Stand a terminal up for every work whose row has arrived and that stands
/// somewhere on this island.
pub fn spawn(
    mut commands: Commands,
    net: NonSend<Net>,
    world: Option<Res<WorldId>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    have: Query<&WorkMark>,
) {
    let Some(world) = world else { return };
    let core = &net.session.core;
    for (k, w) in core.arc.known() {
        if have.iter().any(|m| m.0 as usize == k) {
            continue;
        }
        let Some((x, _, z)) = sim_core::spot::world(&world.haven, &w.def.spot) else {
            continue;
        };
        let y = sim_core::terrain::ground(world.seed, &world.haven, x, z);
        let stone = materials.add(StandardMaterial {
            base_color: Color::srgb(0.06, 0.06, 0.07),
            perceptual_roughness: 0.35,
            ..default()
        });
        let iron = materials.add(StandardMaterial {
            base_color: Color::srgb(0.18, 0.16, 0.15),
            metallic: 0.8,
            perceptual_roughness: 0.5,
            ..default()
        });
        let gold = materials.add(StandardMaterial {
            base_color: Color::srgb(0.83, 0.64, 0.25),
            metallic: 1.0,
            perceptual_roughness: 0.3,
            ..default()
        });
        commands
            .spawn((
                WorldEntity,
                WorkMark(k as u8),
                Transform::from_xyz(x, y, z),
                Visibility::default(),
            ))
            .with_children(|c| {
                c.spawn((
                    Mesh3d(meshes.add(Cuboid::new(1.6, PLINTH_H, 1.6))),
                    MeshMaterial3d(stone.clone()),
                    Transform::from_xyz(0.0, PLINTH_H * 0.5 - 0.1, 0.0),
                ));
                c.spawn((
                    Mesh3d(meshes.add(Cuboid::new(1.1, 0.25, 1.1))),
                    MeshMaterial3d(iron),
                    Transform::from_xyz(0.0, PLINTH_H + 0.02, 0.0),
                ));
                // The ring, standing on edge behind the bowl.
                c.spawn((
                    Mesh3d(meshes.add(Torus::new(1.05, 1.25))),
                    MeshMaterial3d(gold),
                    Transform::from_xyz(0.0, PLINTH_H + 1.3, -0.95)
                        .with_rotation(Quat::from_rotation_x(core::f32::consts::FRAC_PI_2)),
                ));
                c.spawn((
                    WorkFire(k as u8),
                    PointLight {
                        color: FIRE_COLOR,
                        intensity: 0.0,
                        range: FIRE_RANGE_M,
                        shadows_enabled: false,
                        ..default()
                    },
                    Transform::from_xyz(0.0, PLINTH_H + 0.45, 0.0),
                    FireFx {
                        flames: true,
                        flame_dy: -0.35,
                        smoke_dy: 0.5,
                        scale: 1.0,
                        tongues: Tongues::Pit,
                    },
                ));
                c.spawn((
                    WorkGlow(k as u8),
                    PointLight {
                        color: GLOW_OPEN,
                        intensity: 0.0,
                        range: 6.0,
                        shadows_enabled: false,
                        ..default()
                    },
                    Transform::from_xyz(0.0, PLINTH_H + 1.3, -0.6),
                ));
            });
    }
}

/// Light each terminal by its work's state.
#[allow(clippy::type_complexity)]
pub fn light(
    net: NonSend<Net>,
    gain: Res<super::rig::FlameGain>,
    mut fires: Query<(&WorkFire, &mut PointLight), Without<WorkGlow>>,
    mut glows: Query<(&WorkGlow, &mut PointLight), Without<WorkFire>>,
) {
    let arc = &net.session.core.arc;
    for (f, mut light) in fires.iter_mut() {
        let burning = arc
            .works
            .get(f.0 as usize)
            .is_some_and(|w| w.state == WORK_LIT && w.fuel > 0);
        let want = if burning { FIRE_LUMENS * gain.0 } else { 0.0 };
        if light.intensity != want {
            light.intensity = want;
        }
    }
    for (g, mut light) in glows.iter_mut() {
        let Some(w) = arc.works.get(g.0 as usize) else {
            continue;
        };
        let (color, lumens) = match w.state {
            WORK_OPEN => (GLOW_OPEN, GLOW_LUMENS),
            WORK_LIT if w.fuel == 0 => (GLOW_EMBER, GLOW_LUMENS),
            _ => (GLOW_OPEN, 0.0),
        };
        if light.intensity != lumens {
            light.intensity = lumens;
        }
        if light.color != color {
            light.color = color;
        }
    }
}
