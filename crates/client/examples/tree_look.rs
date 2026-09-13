//! `tree_look` — a bench, not a gate: every generated tree's near pair, its
//! far hull, and the two drawn together, shot headless from arm's length.
//!
//! ```text
//! cargo run --release -p client --features render --example tree_look -- <outdir>
//! ```
//!
//! Written 2026-09-13 to answer a question the operator's frame asked and
//! nothing in `tests/` can: what does a broadleaf look like five metres away,
//! what does its hull look like there, and is a hull drawn OVER its own pair
//! the picture the operator took. Three rows of the six variants — row 0 the
//! pair alone, row 1 the hull alone, row 2 both, which is what a tree looks
//! like when neither LOD is culled — and a camera that walks a fixed list of
//! poses, settling a fixed number of FRAMES at each (never a clock; under
//! lavapipe a frame is about a second and says nothing about a GPU).
//!
//! Headless, on a box with no display:
//! ```text
//! Xvfb :99 -screen 0 1280x720x24 &
//! VK_DRIVER_FILES=/usr/share/vulkan/icd.d/lvp_icd.json DISPLAY=:99 WGPU_BACKEND=vulkan \
//!   target/release/examples/tree_look /tmp/shots
//! ```
//!
//! **Not a gate and never will be** (`CLAUDE.md`: the visual gate is a person
//! looking, and this is the thing they look at). The materials are stand-ins
//! for `props.rs`'s — a flat bark colour instead of the photograph — so what a
//! frame here settles is SHAPE and the LOD pair's relationship, not colour.

use std::path::PathBuf;

use bevy::prelude::*;
use bevy::render::view::screenshot::{save_to_disk, Screenshot};

use client::render::tree::{conifer, impostor_of, needle_image, species_of, CONIFER_POOL};

/// Row depths: the pair, the hull, both.
const ROW_Z: [f32; 3] = [0.0, -16.0, -32.0];
/// Metres between variants along X.
const PITCH_M: f32 = 9.0;
/// Frames held at a pose before its shot, and after the last shot before exit.
const SETTLE_FRAMES: u32 = 14;
const TAIL_FRAMES: u32 = 24;

#[derive(Resource)]
struct Out(PathBuf);

#[derive(Resource, Default)]
struct Shots {
    frame: u32,
    step: usize,
    last_shot_frame: Option<u32>,
}

#[derive(Component)]
struct Eye;

/// `(label, eye, look-at)`.
fn poses() -> Vec<(String, Vec3, Vec3)> {
    let mut v = Vec::new();
    let names = ["pair", "hull", "both"];
    for (ri, z) in ROW_Z.iter().enumerate() {
        for (species, var) in [("conifer", 0usize), ("broadleaf", 3usize)] {
            let x = var as f32 * PITCH_M;
            // Arm's length: 5 m back, eye at 1.6 m, looking a little up into the crown.
            v.push((
                format!("{}_{}_5m", names[ri], species),
                Vec3::new(x + 1.2, 1.6, z + 5.0),
                Vec3::new(x, 2.6, *z),
            ));
        }
        // The whole row from 26 m.
        v.push((
            format!("{}_wide", names[ri]),
            Vec3::new(2.5 * PITCH_M, 3.5, z + 26.0),
            Vec3::new(2.5 * PITCH_M, 3.0, *z),
        ));
    }
    v
}

fn main() -> AppExit {
    let Some(dir) = std::env::args().nth(1) else {
        eprintln!("tree_look <outdir>");
        return AppExit::from_code(2);
    };
    let dir = PathBuf::from(dir);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("tree_look: {}: {e}", dir.display());
        return AppExit::from_code(2);
    }
    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "gates — tree_look".into(),
            resolution: (1280u32, 720u32).into(),
            ..default()
        }),
        ..default()
    }));
    app.insert_resource(ClearColor(Color::srgb(0.55, 0.65, 0.80)));
    app.insert_resource(Out(dir));
    app.insert_resource(Shots::default());
    app.add_systems(Startup, stage);
    app.add_systems(Update, shoot);
    app.run()
}

fn stage(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let first = poses().remove(0);
    commands.spawn((
        Eye,
        Camera3d::default(),
        Projection::Perspective(PerspectiveProjection {
            fov: 70.0_f32.to_radians(),
            near: 0.05,
            far: 400.0,
            ..default()
        }),
        Transform::from_translation(first.1).looking_at(first.2, Vec3::Y),
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: 20_000.0,
            shadows_enabled: true,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(EulerRot::YXZ, -0.7, -0.9, 0.0)),
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: 3_000.0,
            shadows_enabled: false,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(EulerRot::YXZ, 2.4, -0.5, 0.0)),
    ));
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(400.0, 400.0))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.30, 0.34, 0.20),
            perceptual_roughness: 0.95,
            ..default()
        })),
        Transform::from_xyz(2.5 * PITCH_M, 0.0, -16.0),
    ));

    let needle_map = images.add(needle_image());
    let bark = materials.add(StandardMaterial {
        base_color: Color::srgb(0.42, 0.31, 0.21),
        perceptual_roughness: 0.92,
        ..default()
    });
    let needle = materials.add(StandardMaterial {
        base_color_texture: Some(needle_map),
        alpha_mode: AlphaMode::Mask(0.5),
        cull_mode: None,
        double_sided: true,
        perceptual_roughness: 0.86,
        ..default()
    });
    // The hull's material: white, vertex-coloured, the shape of `props.rs`'s
    // `foliage` pool.
    let foliage = materials.add(StandardMaterial {
        base_color: Color::WHITE,
        perceptual_roughness: 0.86,
        ..default()
    });

    for v in 0..CONIFER_POOL {
        let (bark_mesh, needle_mesh) = conifer(v);
        let far_mesh = impostor_of(&bark_mesh, &needle_mesh, v);
        println!(
            "tree_look: variant {v} species {} — bark {} tris, needles {} tris, hull {} tris",
            species_of(v),
            client::render::tree::tris(&bark_mesh),
            client::render::tree::tris(&needle_mesh),
            client::render::tree::tris(&far_mesh),
        );
        let bark_h = meshes.add(bark_mesh);
        let needle_h = meshes.add(needle_mesh);
        let far_h = meshes.add(far_mesh);
        let x = v as f32 * PITCH_M;
        for (ri, z) in ROW_Z.iter().enumerate() {
            let at = Transform::from_xyz(x, 0.0, *z);
            let pair = ri == 0 || ri == 2;
            let hull = ri == 1 || ri == 2;
            if pair {
                commands.spawn((Mesh3d(bark_h.clone()), MeshMaterial3d(bark.clone()), at));
                commands.spawn((Mesh3d(needle_h.clone()), MeshMaterial3d(needle.clone()), at));
            }
            if hull {
                commands.spawn((Mesh3d(far_h.clone()), MeshMaterial3d(foliage.clone()), at));
            }
        }
    }
}

fn shoot(
    mut commands: Commands,
    out: Res<Out>,
    mut shots: ResMut<Shots>,
    mut eye: Query<&mut Transform, With<Eye>>,
    mut exit: MessageWriter<AppExit>,
) {
    shots.frame += 1;
    let list = poses();
    if shots.step >= list.len() {
        if let Some(last) = shots.last_shot_frame {
            if shots.frame - last >= TAIL_FRAMES {
                println!("tree_look: {} shots → {}", list.len(), out.0.display());
                exit.write(AppExit::Success);
            }
        }
        return;
    }
    let since = shots.frame - shots.last_shot_frame.unwrap_or(0);
    let (label, at, look) = &list[shots.step];
    if let Ok(mut t) = eye.single_mut() {
        *t = Transform::from_translation(*at).looking_at(*look, Vec3::Y);
    }
    if since < SETTLE_FRAMES {
        return;
    }
    let path = out.0.join(format!("{label}.png"));
    println!("tree_look: shot → {}", path.display());
    commands
        .spawn(Screenshot::primary_window())
        .observe(save_to_disk(path));
    shots.last_shot_frame = Some(shots.frame);
    shots.step += 1;
}
