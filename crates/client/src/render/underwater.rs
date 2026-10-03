//! What the eye sees once it is under the sea: the water's own fog
//! (`sky::underwater_fog`) over everything near, and past it the water
//! itself — never the sky, the clouds or the far shore.
//!
//! `DistanceFog` only reaches meshes. The sky is a `Skybox` and the
//! atmosphere, neither of which it touches, so with the fog alone the eye
//! under the surface still saw clouds, the sun and the island's trees a
//! hundred metres off. This is a shell around the eye at [`SHELL_R_M`],
//! drawn in the fog's own colour and shown only under water: what the fog
//! has not taken by then, the shell covers, and beyond it nothing is drawn.
//! A little lighter toward the surface and darker toward the deep, the way
//! water looks from inside it.

use bevy::camera::visibility::NoFrustumCulling;
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::pbr::DistanceFog;
use bevy::prelude::*;

use super::rig::EyeCam;
use super::weather::WeatherNow;
use super::Eye;

/// The shell's radius. The fog leaves under 1 % of green there
/// (`sky::UNDERWATER_CLARITY` of `water::EXTINCT`, ×60 m), so the seam
/// between the fogged seabed and the shell does not show.
pub const SHELL_R_M: f32 = 60.0;
/// The shell's brightness straight up and straight down, against the fog's
/// colour at the horizon.
pub const SHELL_UP: f32 = 1.8;
pub const SHELL_DOWN: f32 = 0.45;

/// The shell around the eye.
#[derive(Component)]
pub struct UnderShell;

/// The shell's tint for a direction's height `y` (−1 down … 1 up): the
/// fog's colour level with the eye, lighter above it, darker below.
pub fn shade(y: f32) -> f32 {
    if y >= 0.0 {
        1.0 + (SHELL_UP - 1.0) * y
    } else {
        1.0 + (1.0 - SHELL_DOWN) * y
    }
}

fn shell_mesh() -> Mesh {
    let mut mesh = Sphere::new(SHELL_R_M).mesh().uv(32, 18);
    let colors: Vec<[f32; 4]> = match mesh.attribute(Mesh::ATTRIBUTE_POSITION) {
        Some(bevy::mesh::VertexAttributeValues::Float32x3(p)) => p
            .iter()
            .map(|v| {
                let s = shade(v[1] / SHELL_R_M);
                [s, s, s, 1.0]
            })
            .collect(),
        _ => Vec::new(),
    };
    if !colors.is_empty() {
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    }
    mesh
}

/// Spawn the shell, hidden. Runs with the rig, on entering the world.
pub fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        super::WorldEntity,
        UnderShell,
        Mesh3d(meshes.add(shell_mesh())),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::BLACK,
            unlit: true,
            // Its colour is the fog's already; fogging it again would flatten
            // the gradient back to one tone.
            fog_enabled: false,
            cull_mode: None,
            ..default()
        })),
        Transform::default(),
        Visibility::Hidden,
        NoFrustumCulling,
        NotShadowCaster,
        NotShadowReceiver,
    ));
}

/// Show the shell while the eye is under water, centred on it and in the
/// colour `rig::day_night` gave the fog this frame. The material is written
/// only when that colour moves, since a write rebuilds its bind group.
pub fn drive(
    weather: Res<WeatherNow>,
    eye: Res<Eye>,
    fog: Query<&DistanceFog, With<EyeCam>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut shell: Query<
        (
            &mut Transform,
            &mut Visibility,
            &MeshMaterial3d<StandardMaterial>,
        ),
        With<UnderShell>,
    >,
) {
    let Ok((mut t, mut vis, mat)) = shell.single_mut() else {
        return;
    };
    let under = weather.underwater;
    let want = if under {
        Visibility::Visible
    } else {
        Visibility::Hidden
    };
    if *vis != want {
        *vis = want;
    }
    if !under {
        return;
    }
    t.translation = eye.pos;
    let Ok(fog) = fog.single() else {
        return;
    };
    let c = fog.color.to_linear();
    if let Some(m) = materials.get(&mat.0) {
        let old = m.base_color.to_linear();
        let moved =
            (old.red - c.red).abs() + (old.green - c.green).abs() + (old.blue - c.blue).abs();
        if moved < 1e-4 {
            return;
        }
    }
    if let Some(m) = materials.get_mut(&mat.0) {
        m.base_color = Color::LinearRgba(c);
    }
}
