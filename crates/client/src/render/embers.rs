//! The Verdigris' embers: green sparks drifting up around the eye.
//!
//! The rain's machinery turned over (`rain.rs`): the same mesh of streak
//! quads wrapped into a box around the camera by the clock, the same shader,
//! a second material — so it is one more draw and no new pipeline, on WebGL2
//! too. The streaks are short and rise slowly on the wind, and their colour
//! is HDR green, so they read as glowing without bloom (off on Low and in a
//! browser). How many are alive is how deep the eye stands in the Verdigris.

use bevy::camera::visibility::NoFrustumCulling;
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::prelude::*;

use super::rain::{drop_mesh, RainMaterial, RainParams};
use super::weather::WeatherNow;

/// Embers in the mesh — the most the heart of the Verdigris draws.
pub const EMBERS: u32 = if cfg!(webgl2) { 300 } else { 700 };
/// Half the box they drift in around the eye, metres.
pub const BOX_HALF: Vec3 = Vec3::new(11.0, 6.0, 11.0);
/// How fast an ember rises, m/s, and how much of the wind it rides.
const RISE_MS: f32 = 0.55;
const WIND_SHARE: f32 = 0.7;
/// Its streak, metres: a glint, not a line.
const STREAK_M: f32 = 0.07;
/// Its colour, linear HDR: green fire, bright enough to glow unaided. **(knob)**
pub const EMBER_RGB: [f32; 3] = [0.5, 2.6, 0.35];

/// The embers' entity.
#[derive(Component)]
pub struct Embers;

/// The handle `drive` writes.
#[derive(Resource)]
pub struct EmberHandle(pub Handle<RainMaterial>);

/// What the embers look like in this weather: none outside the Verdigris.
pub fn params_for(w: &WeatherNow) -> RainParams {
    let k = (w.fel - 0.25) / 0.5;
    let k = k.clamp(0.0, 1.0);
    let alive = if w.underwater {
        0.0
    } else {
        k * k * (3.0 - 2.0 * k)
    };
    let wind = w.wind_dir * (1.0 + 7.0 * w.wind) * WIND_SHARE;
    RainParams {
        fall: Vec3::new(wind.x, RISE_MS, wind.y).extend(STREAK_M),
        box_: BOX_HALF.extend(alive),
        look: Vec4::new(EMBER_RGB[0], EMBER_RGB[1], EMBER_RGB[2], 0.0),
    }
}

/// Build the embers. Runs with the rain, on entering the world.
pub fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<RainMaterial>>,
) {
    let material = materials.add(RainMaterial {
        params: RainParams::default(),
    });
    commands.spawn((
        super::WorldEntity,
        Embers,
        Mesh3d(meshes.add(drop_mesh(EMBERS))),
        MeshMaterial3d(material.clone()),
        Transform::default(),
        NoFrustumCulling,
        NotShadowCaster,
        NotShadowReceiver,
    ));
    commands.insert_resource(EmberHandle(material));
}

/// Move the embers with the weather and the Verdigris. Writes only when
/// something moved, since a write re-uploads the material.
pub fn drive(
    weather: Res<WeatherNow>,
    handle: Option<Res<EmberHandle>>,
    mut materials: ResMut<Assets<RainMaterial>>,
) {
    let Some(handle) = handle else {
        return;
    };
    let want = params_for(&weather);
    let stale = materials.get(&handle.0).is_some_and(|m| m.params != want);
    if stale {
        if let Some(m) = materials.get_mut(&handle.0) {
            m.params = want;
        }
    }
}
