//! Fires on the ground — what a fire arrow leaves where it lands (`sim-core`'s
//! `fire.rs`). The server says where each burns and until when (`SUB_FIRE`,
//! wire v94) and `ClientCore::fires` keeps them; this hangs the fire pit's own
//! flames, at a fraction of its size, and a warm light on the nearest few.
//!
//! A fixed pool, spawned once, for `tracer.rs`'s reason. The flames are
//! `fx::world::fires`', drawn for every `FireFx` whose light is on, so a fire
//! here is a light turned up and nothing else.

use bevy::prelude::*;

use super::fx::world::FireFx;
use super::structures::FIRE_COLOR;
use super::{Eye, Net};
use sim_core::movement::{POS_XZ_Q, POS_Y_Q};

/// Fires drawn at once, the nearest to the eye.
pub const FIRES_DRAWN: usize = 8;
/// A fire arrow's fire, as a fraction of a fire pit's.
const SCALE: f32 = 0.45;
/// Its light: a pit's at its size, near enough.
const LUMENS: f32 = 350.0;
const RANGE_M: f32 = 4.0;
/// The light hangs this far over the ground, the flames start at it.
const LIFT_M: f32 = 0.2;

/// One of the pool's fires — the marker that keeps the fire pits' own
/// lights out of [`draw`]'s query.
#[derive(Component)]
pub struct ArrowFire;

/// The pool.
#[derive(Resource, Default)]
pub struct ArrowFires {
    entities: Vec<Entity>,
}

/// Spawn the pool, dark.
pub fn setup(mut commands: Commands, mut pool: ResMut<ArrowFires>) {
    pool.entities = (0..FIRES_DRAWN)
        .map(|_| {
            commands
                .spawn((
                    ArrowFire,
                    Transform::default(),
                    PointLight {
                        color: FIRE_COLOR,
                        intensity: 0.0,
                        range: RANGE_M,
                        shadows_enabled: false,
                        ..default()
                    },
                    FireFx {
                        flames: true,
                        flame_dy: -LIFT_M + 0.02,
                        smoke_dy: 0.3,
                        scale: SCALE,
                        tongues: super::fx::world::Tongues::Pit,
                    },
                ))
                .id()
        })
        .collect();
}

/// Light the pool on the nearest fires burning, and put the rest out.
pub fn draw(
    pool: Res<ArrowFires>,
    net: NonSend<Net>,
    eye: Res<Eye>,
    mut q: Query<(&mut Transform, &mut PointLight), With<ArrowFire>>,
) {
    let mut near = [(f32::MAX, Vec3::ZERO); FIRES_DRAWN];
    for f in net.session.core.fires() {
        let at = Vec3::new(
            f.qx as f32 * POS_XZ_Q,
            f.qy as f32 * POS_Y_Q + LIFT_M,
            f.qz as f32 * POS_XZ_Q,
        );
        let d = at.distance_squared(eye.pos);
        if let Some(i) = near.iter().position(|n| d < n.0) {
            near.copy_within(i..FIRES_DRAWN - 1, i + 1);
            near[i] = (d, at);
        }
    }
    for (&e, &(d, at)) in pool.entities.iter().zip(near.iter()) {
        let Ok((mut tf, mut light)) = q.get_mut(e) else {
            continue;
        };
        let lit = d < f32::MAX;
        if lit && tf.translation != at {
            tf.translation = at;
        }
        let want = if lit { LUMENS } else { 0.0 };
        if light.intensity != want {
            light.intensity = want;
        }
    }
}
