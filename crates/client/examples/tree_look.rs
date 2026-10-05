//! `tree_look` — a bench, not a gate: every generated tree's near pair, its
//! far card, the two drawn together, and a grove shot the way the game sees a
//! treeline (near pairs in front, cards behind), all headless.
//!
//! ```text
//! cargo run --release -p client --features render,native --example tree_look -- <outdir>
//! ```
//!
//! Headless, on a box with no display:
//! ```text
//! Xvfb :99 -screen 0 1280x720x24 &
//! VK_DRIVER_FILES=/usr/share/vulkan/icd.d/lvp_icd.json DISPLAY=:99 WGPU_BACKEND=vulkan \
//!   target/release/examples/tree_look /tmp/shots
//! ```
//!
//! The materials are stand-ins for `props.rs`'s (no wind, no transmission, a
//! flat bark colour), so a frame here settles shape and the LOD pair's
//! relationship, not final colour.

use std::path::PathBuf;

use bevy::prelude::*;
use bevy::render::view::screenshot::{save_to_disk, Screenshot};

use client::render::far_trees::{self, TreeCardMaterial, TreeCards};
use client::render::tree::{conifer, leaf_image, needle_image, species_of, CONIFER_POOL};

/// Rows sit side by side along X — the pair, the card, both.
const ROW_X: [f32; 3] = [0.0, 80.0, 160.0];
/// Metres between variants along X within a row.
const PITCH_M: f32 = 9.0;
/// Where the grove stands, away from the rows.
const GROVE: Vec3 = Vec3::new(0.0, 0.0, -400.0);
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

#[derive(Resource)]
struct NearMats {
    bark: Handle<StandardMaterial>,
    needle: Handle<StandardMaterial>,
    leaf: Handle<StandardMaterial>,
    pairs: Vec<(Handle<Mesh>, Handle<Mesh>)>,
}

#[derive(Component)]
struct Eye;

/// `(label, eye, look-at)`.
fn poses() -> Vec<(String, Vec3, Vec3)> {
    let mut v = Vec::new();
    let names = ["pair", "card", "both"];
    for (ri, rx) in ROW_X.iter().enumerate() {
        for (species, var) in [("conifer", 0usize), ("broadleaf", 3usize)] {
            let x = rx + var as f32 * PITCH_M;
            v.push((
                format!("{}_{}_6m", names[ri], species),
                Vec3::new(x + 1.2, 1.6, 6.0),
                Vec3::new(x, 6.5, 0.0),
            ));
            v.push((
                format!("{}_{}_18m", names[ri], species),
                Vec3::new(x + 2.0, 1.6, 18.0),
                Vec3::new(x, 6.5, 0.0),
            ));
        }
        v.push((
            format!("{}_wide", names[ri]),
            Vec3::new(rx + 2.5 * PITCH_M, 5.0, 45.0),
            Vec3::new(rx + 2.5 * PITCH_M, 6.5, 0.0),
        ));
    }
    // Under a crown, looking up — the near canopy as a player sees it.
    v.push((
        "under_conifer".into(),
        Vec3::new(1.5, 1.6, 1.5),
        Vec3::new(-0.5, 9.0, -1.0),
    ));
    // The grove: eye height, then from a rise.
    v.push((
        "grove_eye".into(),
        GROVE + Vec3::new(0.0, 1.6, 0.0),
        GROVE + Vec3::new(0.0, 8.0, -100.0),
    ));
    v.push((
        "grove_rise".into(),
        GROVE + Vec3::new(0.0, 25.0, 20.0),
        GROVE + Vec3::new(0.0, 0.0, -150.0),
    ));
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
    // `TREE_LOOK_ATLAS=1`: also write the far-card atlas's level 0 as a PAM
    // (`=only` writes it and exits, for iterating on the bake alone).
    if let Ok(mode) = std::env::var("TREE_LOOK_ATLAS") {
        let (img, _) = far_trees::bake_atlas();
        let (w, h) = (
            img.texture_descriptor.size.width,
            img.texture_descriptor.size.height,
        );
        let data = img.data.as_deref().unwrap_or_default();
        let mut pam =
            format!("P7\nWIDTH {w}\nHEIGHT {h}\nDEPTH 4\nMAXVAL 255\nTUPLTYPE RGB_ALPHA\nENDHDR\n")
                .into_bytes();
        pam.extend_from_slice(&data[..(w * h * 4) as usize]);
        if let Err(e) = std::fs::write(dir.join("atlas.pam"), pam) {
            eprintln!("tree_look: atlas: {e}");
        }
        if mode == "only" {
            return AppExit::Success;
        }
    }
    let assets = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: "gates — tree_look".into(),
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
    app.add_plugins(MaterialPlugin::<TreeCardMaterial>::default());
    app.insert_resource(ClearColor(Color::srgb(0.55, 0.65, 0.80)));
    app.insert_resource(Out(dir));
    app.insert_resource(Shots::default());
    app.add_systems(Startup, (stage, far_trees::init));
    app.add_systems(Update, (place_cards, shoot));
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
            far: 1000.0,
            ..default()
        }),
        DistanceFog {
            color: Color::srgb(0.62, 0.70, 0.80),
            falloff: FogFalloff::from_visibility(1400.0),
            ..default()
        },
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
    commands.insert_resource(GlobalAmbientLight {
        color: Color::srgb(0.75, 0.82, 0.95),
        brightness: 600.0,
        ..default()
    });
    let ground = materials.add(StandardMaterial {
        base_color: Color::srgb(0.30, 0.34, 0.20),
        perceptual_roughness: 0.95,
        ..default()
    });
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(600.0, 400.0))),
        MeshMaterial3d(ground.clone()),
        Transform::from_xyz(80.0 + 2.5 * PITCH_M, 0.0, 0.0),
    ));
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(800.0, 800.0))),
        MeshMaterial3d(ground),
        Transform::from_translation(GROVE + Vec3::new(0.0, 0.0, -300.0)),
    ));

    let needle_map = images.add(needle_image());
    let leaf_map = images.add(leaf_image());
    let bark = materials.add(StandardMaterial {
        base_color: Color::srgb(0.42, 0.31, 0.21),
        perceptual_roughness: 0.92,
        ..default()
    });
    let g = client::render::tree::NEEDLE_MAP_GAIN;
    let spec = client::render::tree::CANOPY_SPECULAR_TINT;
    let needle = materials.add(StandardMaterial {
        base_color: Color::linear_rgb(g, g, g),
        base_color_texture: Some(needle_map),
        alpha_mode: AlphaMode::Mask(0.5),
        cull_mode: None,
        double_sided: true,
        perceptual_roughness: 0.86,
        specular_tint: Color::linear_rgb(spec, spec, spec),
        ..default()
    });
    let leaf = materials.add(StandardMaterial {
        base_color_texture: Some(leaf_map),
        specular_tint: Color::linear_rgb(spec, spec, spec),
        alpha_mode: AlphaMode::Mask(0.5),
        cull_mode: None,
        double_sided: true,
        perceptual_roughness: 0.86,
        ..default()
    });
    let mut pairs = Vec::new();
    for v in 0..CONIFER_POOL {
        let (bark_mesh, needle_mesh) = conifer(v);
        pairs.push((meshes.add(bark_mesh), meshes.add(needle_mesh)));
    }
    commands.insert_resource(NearMats {
        bark,
        needle,
        leaf,
        pairs,
    });
}

/// Once the cards are baked: the rows, and the grove.
fn place_cards(
    mut commands: Commands,
    cards: Option<Res<TreeCards>>,
    near: Option<Res<NearMats>>,
    mut done: Local<bool>,
) {
    let (Some(cards), Some(near)) = (cards, near) else {
        return;
    };
    if *done {
        return;
    }
    *done = true;
    let spawn_pair = |commands: &mut Commands, v: usize, at: Transform| {
        let card = if species_of(v) == 0 {
            &near.needle
        } else {
            &near.leaf
        };
        let (b, n) = &near.pairs[v];
        commands.spawn((Mesh3d(b.clone()), MeshMaterial3d(near.bark.clone()), at));
        commands.spawn((Mesh3d(n.clone()), MeshMaterial3d(card.clone()), at));
    };
    let spawn_card = |commands: &mut Commands, v: usize, tint: usize, at: Transform| {
        commands.spawn((
            Mesh3d(cards.ring_meshes[v].clone()),
            MeshMaterial3d(cards.ring_materials[tint].clone()),
            at,
        ));
    };
    for v in 0..CONIFER_POOL {
        for (ri, rx) in ROW_X.iter().enumerate() {
            let at = Transform::from_xyz(rx + v as f32 * PITCH_M, 0.0, 0.0);
            if ri == 0 || ri == 2 {
                spawn_pair(&mut commands, v, at);
            }
            if ri == 1 || ri == 2 {
                spawn_card(&mut commands, v, 1, at);
            }
        }
    }
    // The grove: pairs to 60 m, cards past it, mostly conifers.
    let mut rng = fastrand::Rng::with_seed(7);
    for _ in 0..900 {
        let d = 12.0 + rng.f32().powf(0.7) * 330.0;
        let a = (rng.f32() - 0.5) * 2.2;
        let at = GROVE + Vec3::new(a.sin() * d, 0.0, -a.cos() * d);
        let v = if rng.f32() < 0.75 {
            rng.usize(0..3)
        } else {
            rng.usize(3..CONIFER_POOL)
        };
        let t = Transform {
            translation: at,
            rotation: Quat::from_rotation_y(rng.f32() * std::f32::consts::TAU),
            scale: Vec3::splat(0.85 + 0.3 * rng.f32()),
        };
        if d < 60.0 {
            spawn_pair(&mut commands, v, t);
        } else {
            spawn_card(&mut commands, v, rng.usize(0..4), t);
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
