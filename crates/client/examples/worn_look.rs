//! `worn_look` — a bench, not a gate: the player rig wearing each piece
//! `render/worn.rs` can draw, front and back, in the bind pose, headless.
//!
//! ```text
//! cargo build --release -p client --features render,native --example worn_look
//! Xvfb :99 -screen 0 1280x720x24 &
//! VK_DRIVER_FILES=/usr/share/vulkan/icd.d/lvp_icd.json DISPLAY=:99 WGPU_BACKEND=vulkan \
//!   target/release/examples/worn_look /tmp/shots
//! ```

use std::path::PathBuf;

use bevy::mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes};
use bevy::prelude::*;
use bevy::render::view::screenshot::{save_to_disk, Screenshot};

use client::render::bodies::Body;
use client::render::worn::{hang, WornKit, WornRig};

/// What each body in the row wears, left to right.
const OUTFITS: [&[&str]; 4] = [
    &[],
    &["Burlap Hood", "Burlap Tunic"],
    &["Bone Helmet", "Hide Poncho"],
    &["Burlap Hood", "Roadsign Vest"],
];
const PITCH_M: f32 = 1.4;
const SETTLE_FRAMES: u32 = 40;

#[derive(Resource)]
struct Out(PathBuf);

#[derive(Resource, Default)]
struct Shots {
    frame: u32,
    step: usize,
    dressed: bool,
}

#[derive(Component)]
struct Eye;

fn main() -> AppExit {
    let Some(dir) = std::env::args().nth(1) else {
        eprintln!("worn_look <outdir>");
        return AppExit::from_code(2);
    };
    let dir = PathBuf::from(dir);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("worn_look: {e}");
        return AppExit::from_code(2);
    }
    let assets = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: "gates — worn_look".into(),
                    resolution: (1280u32, 720u32).into(),
                    ..default()
                }),
                ..default()
            })
            .set(AssetPlugin {
                file_path: assets.to_string_lossy().into_owned(),
                ..default()
            }),
    );
    app.insert_resource(ClearColor(Color::srgb(0.62, 0.68, 0.74)));
    app.insert_resource(Out(dir));
    app.insert_resource(Shots::default());
    app.add_systems(Startup, (client::render::worn::load, stage));
    app.add_systems(Update, (client::render::worn::bind, dress, shoot).chain());
    app.run()
}

fn stage(
    mut commands: Commands,
    assets: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        Camera3d::default(),
        Eye,
        Transform::from_xyz(2.1, 1.3, 4.2).looking_at(Vec3::new(2.1, 1.0, 0.0), Vec3::Y),
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: 9_000.0,
            shadows_enabled: true,
            ..default()
        },
        Transform::from_xyz(3.0, 6.0, 4.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    commands.insert_resource(GlobalAmbientLight {
        brightness: 600.0,
        ..default()
    });
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(20.0, 20.0))),
        MeshMaterial3d(materials.add(Color::srgb(0.35, 0.38, 0.30))),
    ));
    let scene = assets.load(GltfAssetLabel::Scene(0).from_asset("models/stumpy.glb"));
    for i in 0..OUTFITS.len() {
        // Something has to own an `AnimationPlayer` for `worn::bind` to find
        // the body; the scene's own players do, once it spawns.
        commands.spawn((
            Body(i as u32 + 1),
            SceneRoot(scene.clone()),
            Transform::from_xyz(i as f32 * PITCH_M, 0.0, 0.0),
        ));
    }
}

fn dress(
    mut commands: Commands,
    kit: Option<Res<WornKit>>,
    binds: Res<Assets<SkinnedMeshInverseBindposes>>,
    skins: Query<&SkinnedMesh>,
    mut bodies: Query<(&Body, &mut WornRig)>,
    mut shots: ResMut<Shots>,
) {
    let Some(kit) = kit else { return };
    let mut all = !bodies.is_empty();
    for (body, mut rig) in &mut bodies {
        let Some(skin) = rig.skin.and_then(|e| skins.get(e).ok()) else {
            all = false;
            continue;
        };
        let Some(ibm) = binds.get(&skin.inverse_bindposes) else {
            all = false;
            continue;
        };
        if shots.dressed {
            continue;
        }
        hang(
            &mut commands,
            &kit,
            skin,
            ibm,
            &mut rig,
            OUTFITS[(body.0 as usize - 1) % OUTFITS.len()],
        );
    }
    if all && bodies.iter().count() == OUTFITS.len() {
        shots.dressed = true;
    }
}

fn shoot(
    mut commands: Commands,
    out: Res<Out>,
    mut shots: ResMut<Shots>,
    mut eye: Query<&mut Transform, With<Eye>>,
    mut exit: MessageWriter<AppExit>,
) {
    if !shots.dressed {
        return;
    }
    shots.frame += 1;
    let mid = Vec3::new(PITCH_M * 1.5, 1.0, 0.0);
    let poses = [
        ("front", mid + Vec3::new(0.0, 0.4, 4.6)),
        ("back", mid + Vec3::new(0.0, 0.4, -4.6)),
        ("side", mid + Vec3::new(-5.0, 0.5, 1.5)),
        ("heads", Vec3::new(PITCH_M * 2.0, 1.75, 1.6)),
    ];
    if shots.step >= poses.len() {
        if shots.frame > SETTLE_FRAMES {
            println!("worn_look: {} shots → {}", poses.len(), out.0.display());
            exit.write(AppExit::Success);
        }
        return;
    }
    let (label, at) = poses[shots.step];
    let look = if label == "heads" {
        Vec3::new(PITCH_M * 2.0, 1.45, 0.0)
    } else {
        mid
    };
    if let Ok(mut t) = eye.single_mut() {
        *t = Transform::from_translation(at).looking_at(look, Vec3::Y);
    }
    if shots.frame < SETTLE_FRAMES {
        return;
    }
    let path = out.0.join(format!("{label}.png"));
    commands
        .spawn(Screenshot::primary_window())
        .observe(save_to_disk(path));
    shots.frame = SETTLE_FRAMES / 2;
    shots.step += 1;
}
