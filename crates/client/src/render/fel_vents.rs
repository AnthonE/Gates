//! The Verdigris' vents, drawn: green fire out of the ground wherever
//! `terrain::fel_vent` puts one. Those are the cells the sim reads for the
//! meter's spike (`exposure::corrode`), so the fire a player sees is the one
//! that burns them. Particles in the fire's own pool and two pooled lights
//! on the nearest; nothing here touches the sim.

use bevy::prelude::*;
use sim_core::terrain::{self, FEL_VENT_CELL_M};

use super::fx::pool::{Particle, Pool};
use super::fx::{atlas, Fx};
use super::rig::FlameGain;
use super::{Eye, WorldEntity, WorldId};

/// Vents burn within this range of the eye, metres…
pub const VENT_FX_M: f32 = 45.0;
/// …the nearest this many, so a field of them does not spend the pool.
pub const VENT_FX_MAX: usize = 4;
/// Vent cells searched either side of the eye's: enough to cover the range
/// from anywhere in its own.
const VENT_SCAN: i32 = (VENT_FX_M / FEL_VENT_CELL_M) as i32 + 1;
/// Tongues and embers a second, per vent: a pit of fire wider than a camp's.
const VENT_FLAME_RATE: f32 = 36.0;
const VENT_EMBER_RATE: f32 = 9.0;
/// How far from its middle a tongue may leave, metres: under the reach that
/// burns (`terrain::FEL_VENT_REACH_M`), so what burns you is what you see.
const VENT_MOUTH_M: f32 = 0.9;

/// The fire's green, linear HDR: hot at the root, gone to nothing — the fel
/// beasts' fire (`beasts.rs`).
const VENT_HOT: [f32; 4] = [0.45, 2.6, 0.32, 0.9];
const VENT_COOL: [f32; 4] = [0.04, 0.7, 0.08, 0.0];
const VENT_SPARK: [f32; 4] = [1.2, 6.0, 0.9, 1.0];

/// The light a vent throws: a campfire's (`structures::FIRE_LUMENS`), green
/// and reaching twice as far, since the ground round it is black.
pub const FEL_FIRE_COLOR: Color = Color::srgb(0.35, 1.0, 0.25);
const VENT_LUMENS: f32 = 1_400.0;
const VENT_RANGE_M: f32 = 12.0;
const VENT_LIGHT_LIFT_M: f32 = 0.8;
/// Lights lent to the nearest vents.
const VENT_LIGHTS: usize = 2;

/// One of the pooled vent lights.
#[derive(Component)]
pub struct VentLight;

/// The vents round the eye, found when it crosses into another vent cell:
/// the search taps the terrain, and the answer cannot change inside a cell.
#[derive(Default)]
pub struct VentCache {
    key: Option<(u64, i32, i32)>,
    vents: Vec<Vec3>,
}

/// Hang the pooled lights, dark. Runs on entering the world.
pub fn setup(mut commands: Commands) {
    for _ in 0..VENT_LIGHTS {
        commands.spawn((
            WorldEntity,
            VentLight,
            PointLight {
                color: FEL_FIRE_COLOR,
                intensity: 0.0,
                range: VENT_RANGE_M,
                shadows_enabled: false,
                ..default()
            },
            Transform::default(),
        ));
    }
}

/// How many of a `rate`-per-second stream fall in `dt` (`fx::world`'s).
fn count(pool: &mut Pool, rate: f32, dt: f32) -> usize {
    let x = rate * dt;
    let whole = x.floor();
    whole as usize + usize::from(pool.roll() < x - whole)
}

/// Keep the vents near the eye burning, and the nearest lit.
pub fn drive(
    time: Res<Time>,
    eye: Res<Eye>,
    world: Res<WorldId>,
    gain: Res<FlameGain>,
    mut fx: ResMut<Fx>,
    mut cache: Local<VentCache>,
    mut lights: Query<(&mut PointLight, &mut Transform), With<VentLight>>,
) {
    let vx = (eye.pos.x / FEL_VENT_CELL_M).floor() as i32;
    let vz = (eye.pos.z / FEL_VENT_CELL_M).floor() as i32;
    let key = Some((world.seed, vx, vz));
    if cache.key != key {
        cache.key = key;
        cache.vents.clear();
        for dz in -VENT_SCAN..=VENT_SCAN {
            for dx in -VENT_SCAN..=VENT_SCAN {
                if let Some((x, z)) = terrain::fel_vent(world.seed, vx + dx, vz + dz) {
                    let y = terrain::ground(world.seed, &world.haven, x, z);
                    cache.vents.push(Vec3::new(x, y, z));
                }
            }
        }
    }
    // The nearest few in range, by insertion into a fixed list.
    let mut near = [(f32::MAX, Vec3::ZERO); VENT_FX_MAX];
    let mut n = 0;
    for &at in &cache.vents {
        let d = at.distance(eye.pos);
        if d > VENT_FX_M {
            continue;
        }
        if let Some(i) = near.iter().position(|v| d < v.0) {
            near.copy_within(i..VENT_FX_MAX - 1, i + 1);
            near[i] = (d, at);
            n = (n + 1).min(VENT_FX_MAX);
        }
    }
    for (lit, (mut light, mut tf)) in lights.iter_mut().enumerate() {
        let want = if lit < n {
            let at = near[lit].1 + Vec3::Y * VENT_LIGHT_LIFT_M;
            if tf.translation != at {
                tf.translation = at;
            }
            VENT_LUMENS * gain.0
        } else {
            0.0
        };
        if light.intensity != want {
            light.intensity = want;
        }
    }
    let dt = time.delta_secs().min(0.1);
    if dt <= 0.0 {
        return;
    }
    let glow = &mut fx.glow;
    for &(_, at) in &near[..n] {
        let base = at + Vec3::Y * 0.05;
        for _ in 0..count(glow, VENT_FLAME_RATE, dt) {
            let off = Vec3::new(glow.signed(), 0.0, glow.signed()) * VENT_MOUTH_M;
            let life = 0.45 + 0.35 * glow.roll();
            let size = 0.22 + 0.14 * glow.roll();
            let rise = 1.3 + 0.9 * glow.roll();
            let roll = glow.roll() * std::f32::consts::TAU;
            let spin = glow.signed() * 1.5;
            let cell = atlas::PUFF + ((glow.roll() * 3.0) as u8).min(2);
            glow.spawn(Particle {
                left: life,
                life,
                pos: base + off,
                vel: Vec3::new(-off.x * 0.6, rise, -off.z * 0.6),
                size0: size,
                size1: size * 0.25,
                c0: VENT_HOT,
                c1: VENT_COOL,
                gravity: -0.6,
                drag: 1.2,
                roll,
                spin,
                cell,
                ..Particle::default()
            });
        }
        for _ in 0..count(glow, VENT_EMBER_RATE, dt) {
            let life = 1.4 + glow.roll();
            let pos = base
                + Vec3::new(glow.signed(), 0.0, glow.signed()) * (VENT_MOUTH_M * 0.6)
                + Vec3::Y * 0.3;
            let vel = Vec3::new(
                glow.signed() * 0.4,
                1.6 + 1.4 * glow.roll(),
                glow.signed() * 0.4,
            );
            glow.spawn(Particle {
                left: life,
                life,
                pos,
                vel,
                size0: 0.014,
                size1: 0.006,
                c0: VENT_SPARK,
                c1: VENT_COOL,
                gravity: -0.3,
                drag: 0.9,
                cell: atlas::GLOW,
                ..Particle::default()
            });
        }
    }
}
