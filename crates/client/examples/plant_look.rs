//! `plant_look` — a bench, not a gate: the plants the bush column grows
//! (`render/plants.rs`) — the two shrubs, the berry bush in both colours and
//! hemp, then the scenery (a fern, a low pine, dead scrub, wildflowers) — in
//! two rows, close up, and scattered over a meadow, a forest floor and a
//! ridge, all headless.
//!
//! ```text
//! cargo run --release -p client --features render,native --example plant_look -- <outdir>
//! ```
//!
//! Headless, on a box with no display:
//! ```text
//! Xvfb :99 -screen 0 1280x720x24 &
//! VK_DRIVER_FILES=/usr/share/vulkan/icd.d/lvp_icd.json DISPLAY=:99 WGPU_BACKEND=vulkan \
//!   target/release/examples/plant_look /tmp/shots
//! ```
//!
//! The materials are stand-ins for `props.rs`'s (no wind, no transmission),
//! so a frame here settles shape, density and the berries' read, not final
//! colour.

use std::path::PathBuf;

use bevy::prelude::*;
use bevy::render::view::screenshot::{save_to_disk, Screenshot};

use client::render::clutter::FERN_ATLAS;
use client::render::plants::{
    berries, berry_leaves, berry_wood, dead_scrub, fern, hemp, hemp_leaf_image, juniper,
    shrub_leaves, shrub_wood, wildflowers, PLANT_POOL,
};
use client::render::props::BUSH_CARD_ATLAS;

/// Where the row stands, left to right, and the meadow.
const ROW_PITCH_M: f32 = 2.4;
const MEADOW: Vec3 = Vec3::new(0.0, 0.0, -200.0);
const SCENERY_Z: f32 = -4.0;
const FOREST: Vec3 = Vec3::new(220.0, 0.0, -200.0);
const RIDGE: Vec3 = Vec3::new(-220.0, 0.0, -200.0);
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
    let x = |i: f32| i * ROW_PITCH_M;
    let mut v = vec![
        (
            "row_eye".to_string(),
            Vec3::new(x(2.5), 1.6, 6.5),
            Vec3::new(x(2.5), 0.7, 0.0),
        ),
        (
            "shrub_close".to_string(),
            Vec3::new(x(0.0) + 0.6, 1.6, 2.6),
            Vec3::new(x(0.0), 0.6, 0.0),
        ),
        (
            "tall_shrub_close".to_string(),
            Vec3::new(x(1.0) + 0.5, 1.6, 2.8),
            Vec3::new(x(1.0), 1.0, 0.0),
        ),
        (
            "berry_red_close".to_string(),
            Vec3::new(x(2.0) + 0.4, 1.5, 1.9),
            Vec3::new(x(2.0), 0.45, 0.0),
        ),
        (
            "berry_blue_close".to_string(),
            Vec3::new(x(3.0) - 0.4, 1.5, 1.9),
            Vec3::new(x(3.0), 0.45, 0.0),
        ),
        (
            "hemp_close".to_string(),
            Vec3::new(x(4.0) + 0.3, 1.6, 2.0),
            Vec3::new(x(4.0), 0.7, 0.0),
        ),
        (
            "hemp_above".to_string(),
            Vec3::new(x(5.0) + 0.6, 2.2, 1.2),
            Vec3::new(x(5.0), 0.6, 0.0),
        ),
        (
            "meadow_eye".to_string(),
            MEADOW + Vec3::new(0.0, 1.6, 20.0),
            MEADOW + Vec3::new(0.0, 0.5, -20.0),
        ),
        (
            "scenery_row".to_string(),
            Vec3::new(x(2.5), 1.6, SCENERY_Z + 6.5),
            Vec3::new(x(2.5), 0.5, SCENERY_Z),
        ),
        (
            "fern_close".to_string(),
            Vec3::new(x(0.0) + 0.5, 1.6, SCENERY_Z + 2.4),
            Vec3::new(x(0.0), 0.4, SCENERY_Z),
        ),
        (
            "juniper_close".to_string(),
            Vec3::new(x(1.0) + 0.5, 1.6, SCENERY_Z + 2.6),
            Vec3::new(x(1.0), 0.35, SCENERY_Z),
        ),
        (
            "dead_close".to_string(),
            Vec3::new(x(2.0) + 0.3, 1.5, SCENERY_Z + 2.0),
            Vec3::new(x(2.0), 0.4, SCENERY_Z),
        ),
        (
            "flowers_close".to_string(),
            Vec3::new(x(3.5), 1.4, SCENERY_Z + 2.4),
            Vec3::new(x(3.5), 0.3, SCENERY_Z),
        ),
        (
            "forest_floor_eye".to_string(),
            FOREST + Vec3::new(0.0, 1.6, 18.0),
            FOREST + Vec3::new(0.0, 0.4, -10.0),
        ),
        (
            "ridge_eye".to_string(),
            RIDGE + Vec3::new(0.0, 1.6, 18.0),
            RIDGE + Vec3::new(0.0, 0.4, -10.0),
        ),
    ];
    if let Ok(only) = std::env::var("PLANT_LOOK_ONLY") {
        v.retain(|(label, _, _)| label.contains(&only));
    }
    v
}

fn main() -> AppExit {
    let Some(dir) = std::env::args().nth(1) else {
        eprintln!("plant_look <outdir>");
        return AppExit::from_code(2);
    };
    let dir = PathBuf::from(dir);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("plant_look: {}: {e}", dir.display());
        return AppExit::from_code(2);
    }
    let assets = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: "gates — plant_look".into(),
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
    app.add_systems(Startup, stage);
    app.add_systems(Update, shoot);
    app.run()
}

/// A card atlas with the game's coverage-preserving mip chain, decoded here
/// because the bench has no `mipmap` system watching its loads.
fn card_atlas(images: &mut Assets<Image>, file: &str) -> Handle<Image> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets")
        .join(file);
    let bytes = std::fs::read(&path).expect("the atlas");
    let mut img = Image::from_buffer(
        &bytes,
        bevy::image::ImageType::Extension("png"),
        bevy::image::CompressedImageFormats::NONE,
        true,
        bevy::image::ImageSampler::Default,
        bevy::asset::RenderAssetUsages::default(),
    )
    .expect("a PNG");
    let (w, h) = (
        img.texture_descriptor.size.width,
        img.texture_descriptor.size.height,
    );
    let data = img.data.take().expect("pixels");
    if w == h {
        return images.add(client::render::tree::alpha_card(data, w));
    }
    // The fern atlas is two to one, which `alpha_card` does not take: the
    // same chain by hand.
    use client::render::mipmap::{chain, levels, Filter};
    img.data = Some(chain(&data, w, h, Filter::Mask));
    img.texture_descriptor.mip_level_count = levels(w, h);
    img.sampler = bevy::image::ImageSampler::Descriptor(bevy::image::ImageSamplerDescriptor {
        mag_filter: bevy::image::ImageFilterMode::Linear,
        min_filter: bevy::image::ImageFilterMode::Linear,
        mipmap_filter: bevy::image::ImageFilterMode::Linear,
        anisotropy_clamp: 4,
        ..default()
    });
    images.add(img)
}

#[allow(clippy::too_many_arguments)]
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
            color: Color::srgb(0.62, 0.68, 0.74),
            falloff: FogFalloff::from_visibility(600.0),
            ..default()
        },
        Transform::from_translation(first.1).looking_at(first.2, Vec3::Y),
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: 16_000.0,
            // `PLANT_LOOK_NOSHADOW=1`: the sun casts nothing.
            shadows_enabled: std::env::var("PLANT_LOOK_NOSHADOW").is_err(),
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(EulerRot::YXZ, -0.7, -0.8, 0.0)),
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
        brightness: 700.0,
        ..default()
    });
    let ground = materials.add(StandardMaterial {
        base_color: Color::srgb(0.27, 0.30, 0.17),
        perceptual_roughness: 0.95,
        ..default()
    });
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(800.0, 800.0))),
        MeshMaterial3d(ground),
        Transform::from_xyz(0.0, 0.0, -150.0),
    ));

    let atlas = card_atlas(&mut images, BUSH_CARD_ATLAS);
    let leaf = |materials: &mut Assets<StandardMaterial>, tex: Handle<Image>, rough: f32| {
        materials.add(StandardMaterial {
            base_color_texture: Some(tex),
            alpha_mode: AlphaMode::Mask(0.5),
            cull_mode: None,
            double_sided: true,
            perceptual_roughness: rough,
            ..default()
        })
    };
    let bush_leaf = leaf(&mut materials, atlas, 0.88);
    let hemp_leaf = leaf(&mut materials, images.add(hemp_leaf_image()), 0.8);
    let wood = materials.add(StandardMaterial {
        base_color: Color::WHITE,
        perceptual_roughness: 0.85,
        ..default()
    });
    let berry = materials.add(StandardMaterial {
        base_color: Color::WHITE,
        perceptual_roughness: 0.32,
        ..default()
    });
    let fern_leaf = leaf(&mut materials, card_atlas(&mut images, FERN_ATLAS), 0.9);
    let juniper_leaf = {
        let g = client::render::tree::NEEDLE_MAP_GAIN;
        materials.add(StandardMaterial {
            base_color: Color::linear_rgb(g, g, g),
            base_color_texture: Some(images.add(client::render::tree::needle_image())),
            alpha_mode: AlphaMode::Mask(0.5),
            cull_mode: None,
            double_sided: true,
            perceptual_roughness: 0.9,
            ..default()
        })
    };
    let flower = materials.add(StandardMaterial {
        base_color: Color::WHITE,
        perceptual_roughness: 0.6,
        ..default()
    });

    // `(wood, wood material, leaves and their material, berries)` per kind,
    // every pool variant.
    type Kind = (
        Handle<Mesh>,
        Handle<StandardMaterial>,
        Option<(Handle<Mesh>, Handle<StandardMaterial>)>,
        Option<Handle<Mesh>>,
    );
    let mut kinds: Vec<Vec<Kind>> = Vec::new();
    for tall in [false, true] {
        kinds.push(
            (0..PLANT_POOL as u32)
                .map(|v| {
                    (
                        meshes.add(shrub_wood(v, tall)),
                        wood.clone(),
                        Some((meshes.add(shrub_leaves(v, tall)), bush_leaf.clone())),
                        None,
                    )
                })
                .collect(),
        );
    }
    for red in [true, false] {
        kinds.push(
            (0..PLANT_POOL as u32)
                .map(|v| {
                    (
                        meshes.add(berry_wood(v)),
                        wood.clone(),
                        Some((meshes.add(berry_leaves(v)), bush_leaf.clone())),
                        Some(meshes.add(berries(v, red))),
                    )
                })
                .collect(),
        );
    }
    kinds.push(
        (0..PLANT_POOL as u32)
            .map(|v| {
                let (stalk, leaves) = hemp(v);
                (
                    meshes.add(stalk),
                    wood.clone(),
                    Some((meshes.add(leaves), hemp_leaf.clone())),
                    None,
                )
            })
            .collect(),
    );
    // 5: fern, 6: low pine, 7: dead scrub, 8..: wildflowers by palette.
    kinds.push(
        (0..PLANT_POOL as u32)
            .map(|v| {
                let (w, l) = fern(v);
                (
                    meshes.add(w),
                    wood.clone(),
                    Some((meshes.add(l), fern_leaf.clone())),
                    None,
                )
            })
            .collect(),
    );
    kinds.push(
        (0..PLANT_POOL as u32)
            .map(|v| {
                let (w, l) = juniper(v);
                (
                    meshes.add(w),
                    wood.clone(),
                    Some((meshes.add(l), juniper_leaf.clone())),
                    None,
                )
            })
            .collect(),
    );
    kinds.push(
        (0..PLANT_POOL as u32)
            .map(|v| (meshes.add(dead_scrub(v)), wood.clone(), None, None))
            .collect(),
    );
    for p in 0..client::render::plants::FLOWER_PALETTES as u32 {
        kinds.push(
            (0..PLANT_POOL as u32)
                .map(|v| {
                    let (heads, base) = wildflowers(v, p);
                    (
                        meshes.add(heads),
                        flower.clone(),
                        Some((meshes.add(base), bush_leaf.clone())),
                        None,
                    )
                })
                .collect(),
        );
    }
    const FERN: usize = 5;
    const PINE: usize = 6;
    const DEAD: usize = 7;
    const FLOWERS: usize = 8;
    let spawn = |commands: &mut Commands, kind: usize, v: usize, at: Transform| {
        let (w, wm, l, f) = &kinds[kind][v % PLANT_POOL];
        commands.spawn((Mesh3d(w.clone()), MeshMaterial3d(wm.clone()), at));
        if let Some((l, m)) = l {
            commands.spawn((Mesh3d(l.clone()), MeshMaterial3d(m.clone()), at));
        }
        if let Some(f) = f {
            commands.spawn((Mesh3d(f.clone()), MeshMaterial3d(berry.clone()), at));
        }
    };

    // The row: short shrub, tall shrub, red berries, blue berries, hemp, hemp.
    for (i, kind) in [0usize, 1, 2, 3, 4, 4].into_iter().enumerate() {
        let at = Transform::from_xyz(i as f32 * ROW_PITCH_M, 0.0, 0.0)
            .with_rotation(Quat::from_rotation_y(i as f32 * 1.3));
        spawn(&mut commands, kind, i, at);
    }

    // The scenery row: fern, low pine, dead scrub, then wildflowers in three
    // colours, a pitch apart from x = 3 on.
    for (i, kind) in [FERN, PINE, DEAD, FLOWERS, FLOWERS + 2, FLOWERS + 3]
        .into_iter()
        .enumerate()
    {
        let at = Transform::from_xyz(i as f32 * ROW_PITCH_M, 0.0, SCENERY_Z)
            .with_rotation(Quat::from_rotation_y(i as f32 * 1.1));
        spawn(&mut commands, kind, i, at);
    }

    // A forest floor (fern beds and shrubs) and a ridge (low pines and dead
    // scrub), the shares `plants::SCENERY_SHARES` gives each, roughly.
    let mut rng = fastrand::Rng::with_seed(23);
    for (centre, picks) in [
        (FOREST, [FERN, FERN, FERN, 0, 1, DEAD]),
        (RIDGE, [PINE, PINE, PINE, DEAD, DEAD, 0]),
    ] {
        for _ in 0..90 {
            let at = centre + Vec3::new(rng.f32() * 60.0 - 30.0, 0.0, rng.f32() * -50.0 + 12.0);
            let t = Transform {
                translation: at,
                rotation: Quat::from_rotation_y(rng.f32() * std::f32::consts::TAU),
                scale: Vec3::splat(0.9 + 0.2 * rng.f32()),
            };
            spawn(
                &mut commands,
                picks[rng.usize(0..picks.len())],
                rng.usize(0..PLANT_POOL),
                t,
            );
        }
    }

    // The meadow: the sim's Meadow shares, roughly — mostly scenery.
    let mut rng = fastrand::Rng::with_seed(11);
    for _ in 0..140 {
        let at = MEADOW + Vec3::new(rng.f32() * 80.0 - 40.0, 0.0, rng.f32() * -70.0 + 14.0);
        let r = rng.f32();
        let kind = if r < 0.2 {
            2 + rng.usize(0..2)
        } else if r < 0.53 {
            4
        } else if r < 0.75 {
            FLOWERS + rng.usize(0..client::render::plants::FLOWER_PALETTES)
        } else {
            rng.usize(0..2)
        };
        let t = Transform {
            translation: at,
            rotation: Quat::from_rotation_y(rng.f32() * std::f32::consts::TAU),
            scale: Vec3::splat(0.9 + 0.2 * rng.f32()),
        };
        spawn(&mut commands, kind, rng.usize(0..PLANT_POOL), t);
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
                println!("plant_look: {} shots → {}", list.len(), out.0.display());
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
    println!("plant_look: shot → {}", path.display());
    commands
        .spawn(Screenshot::primary_window())
        .observe(save_to_disk(path));
    shots.last_shot_frame = Some(shots.frame);
    shots.step += 1;
}
