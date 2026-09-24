//! `cargo run -p client --features render --bin iconbake -- <out_dir> [stem…]`
//! — photograph the items that have a 3D model, for their inventory icons.
//!
//! Rust's icons are renders of the item's own model on a transparent ground,
//! lit from the upper left and seen from a three-quarter angle. Ours were
//! white game-icons.net silhouettes tinted at the draw, so every cell was
//! the same off-white. This renders the items whose model makes a good
//! picture ([`SUBJECTS`]) the same way Rust's are, and `ci/finish_icons.py`
//! turns the frames into `assets/icons/*.png`. Every other item keeps its
//! silhouette, which the finisher paints.
//!
//! Headless: no window, no winit. The camera draws into an offscreen image
//! with a transparent clear, and the frame is read back with `Screenshot`.
//! Runs on a GPU-less box through Mesa's lavapipe
//! (`apt-get install mesa-vulkan-drivers`).
//!
//! Not a gate and not run by CI. Re-run it when a model in [`SUBJECTS`]
//! changes, then run the finisher:
//!
//! ```text
//! cargo run -p client --features render --bin iconbake -- ci/icons/renders
//! python3 ci/finish_icons.py
//! ```

use std::path::PathBuf;
use std::time::Duration;

use bevy::app::ScheduleRunnerPlugin;
use bevy::camera::primitives::Aabb;
use bevy::camera::{RenderTarget, ScalingMode};
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::render::view::Msaa;
use bevy::window::ExitCondition;
use bevy::winit::WinitPlugin;

/// Edge of the offscreen frame, px. The finisher downsamples to the shipped
/// 128, and four samples per output pixel on top of MSAA is what keeps a
/// spear's thin haft from stair-stepping.
const FRAME_PX: u32 = 512;

/// Frames between placing a subject and shooting it. Frames, not time: the
/// first subject also waits out shader compilation, which lavapipe does on
/// the CPU, and a readback is asynchronous in frames.
const SETTLE_FIRST: u32 = 120;
const SETTLE: u32 = 30;
/// A glTF that has not spawned a mesh by then is reported and skipped.
const LOAD_TIMEOUT: u32 = 900;

/// One icon to photograph.
#[derive(Clone, Copy)]
struct Subject {
    /// The icon's file stem — the item's display name, normalised
    /// (`ui::icons::stem`).
    stem: &'static str,
    /// The model, under `assets/`.
    glb: &'static str,
    /// Model rotation before framing, degrees: yaw about Y, then pitch about
    /// X, then roll about Z (`EulerRot::YXZ`). The camera never leaves its
    /// three-quarter bearing, so this is how a subject shows its best side.
    turn: [f32; 3],
}

const fn s(stem: &'static str, glb: &'static str, turn: [f32; 3]) -> Subject {
    Subject { stem, glb, turn }
}

/// The items photographed rather than painted.
///
/// **Chunky things only, and that was measured rather than assumed.** Every
/// item with a model was rendered on 2026-09-24 and each render put beside
/// its painted silhouette at the 34 px a cell draws it: the boxes, the bag,
/// the bench, the cupboard and the stones read better as photographs; the
/// thin tools (spear, hatchet, pickaxe, hammer, bow) went to a hairline and
/// read worse, and the generated torch and revolver are boxes that a
/// silhouette beats. Those stay painted until a model is chunky enough.
/// Framing is `ci/finish_icons.py`'s, from the pixels, so there is no size
/// here.
const SUBJECTS: &[Subject] = &[
    s("rock", "models/held/rock.glb", [20.0, 0.0, 0.0]),
    s("stone", "models/prop/node_stone.glb", [30.0, 0.0, 0.0]),
    s("metal_ore", "models/prop/node_metal.glb", [30.0, 0.0, 0.0]),
    s(
        "sulfur_ore",
        "models/prop/node_sulfur.glb",
        [30.0, 0.0, 0.0],
    ),
    // The small box shares this model; the finisher draws it smaller.
    s("large_box", "models/deploy/box.glb", [0.0, 0.0, 0.0]),
    s("sleeping_bag", "models/deploy/bag.glb", [-20.0, 0.0, 0.0]),
    s("workbench", "models/deploy/workbench.glb", [0.0, 0.0, 0.0]),
    // The hearth is the tool cupboard, and the render is the cupboard.
    s("hearth", "models/deploy/hearth.glb", [0.0, 0.0, 0.0]),
];

/// Where the camera sits relative to the subject: right, above and in front,
/// Rust's three-quarter view. Normalised at use.
const VIEW_DIR: Vec3 = Vec3::new(0.62, 0.55, 1.0);

#[derive(Resource)]
struct Bake {
    out: PathBuf,
    queue: Vec<Subject>,
    target: Handle<Image>,
    cur: Option<Current>,
    shot: usize,
    failed: Vec<&'static str>,
}

struct Current {
    subject: Subject,
    root: Entity,
    phase: Phase,
    frames: u32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Loading,
    Settling,
    Shooting,
}

/// Set by the readback observer when the frame is on disk.
#[derive(Resource, Default)]
struct Written(bool);

#[derive(Component)]
struct IconCam;

fn main() -> AppExit {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(out) = args.first().cloned() else {
        eprintln!("usage: iconbake <out_dir> [stem…]");
        for s in SUBJECTS {
            eprintln!("  {}", s.stem);
        }
        return AppExit::from_code(2);
    };
    let only: Vec<&String> = args.iter().skip(1).collect();
    let queue: Vec<Subject> = SUBJECTS
        .iter()
        .copied()
        .filter(|s| only.is_empty() || only.iter().any(|o| o.as_str() == s.stem))
        .rev()
        .collect();
    if queue.is_empty() {
        eprintln!("iconbake: no subject matches {only:?}");
        return AppExit::from_code(2);
    }
    let out = PathBuf::from(out);
    if let Err(e) = std::fs::create_dir_all(&out) {
        eprintln!("iconbake: {}: {e}", out.display());
        return AppExit::from_code(2);
    }

    let mut app = App::new();
    let mut assets = AssetPlugin {
        file_path: std::env::current_dir()
            .map(|d| d.join("assets"))
            .unwrap_or_else(|_| PathBuf::from("assets"))
            .to_string_lossy()
            .into_owned(),
        ..default()
    };
    assets.watch_for_changes_override = Some(false);
    app.add_plugins(
        DefaultPlugins
            .build()
            .disable::<bevy::audio::AudioPlugin>()
            .disable::<WinitPlugin>()
            .set(assets)
            .set(WindowPlugin {
                primary_window: None,
                exit_condition: ExitCondition::DontExit,
                close_when_requested: false,
                ..default()
            }),
    )
    .add_plugins(ScheduleRunnerPlugin::run_loop(Duration::from_millis(1)));
    app.insert_resource(ClearColor(Color::NONE));
    // Dim and cool: enough that a shadow side keeps its colour, low enough
    // that the key light still models the form. Bevy's default exposure
    // (EV100 9.7) puts a lit albedo near white at ~3,000 lux, which is what
    // the three lights below are sized against.
    app.insert_resource(GlobalAmbientLight {
        color: Color::srgb(0.92, 0.95, 1.0),
        brightness: 90.0,
        ..default()
    });
    app.insert_resource(Written::default());
    app.insert_resource(Bake {
        out,
        queue,
        target: Handle::default(),
        cur: None,
        shot: 0,
        failed: Vec::new(),
    });
    app.add_systems(Startup, stage);
    app.add_systems(Update, drive);
    app.run()
}

/// The camera, its offscreen target and the studio lights.
fn stage(mut commands: Commands, mut images: ResMut<Assets<Image>>, mut bake: ResMut<Bake>) {
    let target = images.add(Image::new_target_texture(
        FRAME_PX,
        FRAME_PX,
        TextureFormat::Rgba8UnormSrgb,
        None,
    ));
    bake.target = target.clone();
    commands.spawn((
        IconCam,
        Camera3d::default(),
        Camera {
            clear_color: ClearColorConfig::Custom(Color::NONE),
            ..default()
        },
        RenderTarget::Image(target.into()),
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: ScalingMode::Fixed {
                width: 1.0,
                height: 1.0,
            },
            near: 0.0,
            far: 100.0,
            ..OrthographicProjection::default_3d()
        }),
        // ACES over the game's TonyMcMapface: an icon sits on a dark cell at
        // 34 px, and the extra contrast is what keeps it a picture there.
        Tonemapping::AcesFitted,
        Msaa::Sample4,
        Transform::from_translation(VIEW_DIR.normalize() * 10.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    // Key: high and to the camera's left, so every icon is lit from the
    // upper left like Rust's and the silhouettes the finisher shades.
    commands.spawn((
        DirectionalLight {
            illuminance: 3_200.0,
            shadows_enabled: false,
            ..default()
        },
        Transform::from_xyz(-0.6, 1.0, 0.7).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    // Fill from the right, cool and weak, so the shadow side keeps its form.
    commands.spawn((
        DirectionalLight {
            illuminance: 850.0,
            color: Color::srgb(0.80, 0.86, 1.0),
            shadows_enabled: false,
            ..default()
        },
        Transform::from_xyz(1.0, 0.2, 0.4).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    // Rim from behind: separates a dark item from the dark slot it sits in.
    commands.spawn((
        DirectionalLight {
            illuminance: 1_700.0,
            color: Color::srgb(1.0, 0.95, 0.88),
            shadows_enabled: false,
            ..default()
        },
        Transform::from_xyz(0.3, 0.6, -1.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}

#[allow(clippy::too_many_arguments)]
fn drive(
    mut commands: Commands,
    mut bake: ResMut<Bake>,
    mut written: ResMut<Written>,
    assets: Res<AssetServer>,
    children: Query<&Children>,
    bounds: Query<(&Aabb, &GlobalTransform)>,
    mut cam: Query<(&mut Transform, &mut Projection), With<IconCam>>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(mut cur) = bake.cur.take() else {
        let Some(subject) = bake.queue.pop() else {
            if bake.failed.is_empty() {
                println!("iconbake: {} icon(s) → {}", bake.shot, bake.out.display());
                exit.write(AppExit::Success);
            } else {
                eprintln!("iconbake: failed: {:?}", bake.failed);
                exit.write(AppExit::error());
            }
            return;
        };
        let turn = Quat::from_euler(
            EulerRot::YXZ,
            subject.turn[0].to_radians(),
            subject.turn[1].to_radians(),
            subject.turn[2].to_radians(),
        );
        let place = Transform::from_rotation(turn);
        let root = commands
            .spawn((
                SceneRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(subject.glb))),
                place,
            ))
            .id();
        bake.cur = Some(Current {
            subject,
            root,
            phase: Phase::Loading,
            frames: 0,
        });
        return;
    };
    cur.frames += 1;

    match cur.phase {
        Phase::Loading => {
            if let Some((min, max)) = world_bounds(cur.root, &children, &bounds) {
                // Frame it: the corners on the camera's own axes, a square
                // around the larger extent, centred on the middle of both.
                let back = VIEW_DIR.normalize();
                let look = Transform::from_translation(back).looking_at(Vec3::ZERO, Vec3::Y);
                let (right, up) = (look.right().as_vec3(), look.up().as_vec3());
                let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
                for i in 0..8 {
                    let p = Vec3::new(
                        if i & 1 == 0 { min.x } else { max.x },
                        if i & 2 == 0 { min.y } else { max.y },
                        if i & 4 == 0 { min.z } else { max.z },
                    );
                    let q = Vec3::new(p.dot(right), p.dot(up), p.dot(back));
                    lo = lo.min(q);
                    hi = hi.max(q);
                }
                let mid = (lo + hi) * 0.5;
                // Loose on purpose: the finisher crops to the pixels.
                let side = (hi.x - lo.x).max(hi.y - lo.y) * 1.15;
                if let Ok((mut t, mut proj)) = cam.single_mut() {
                    let at = right * mid.x + up * mid.y + back * (hi.z + 5.0);
                    *t = Transform::from_translation(at).looking_to(-back, Vec3::Y);
                    *proj = Projection::Orthographic(OrthographicProjection {
                        scaling_mode: ScalingMode::Fixed {
                            width: side,
                            height: side,
                        },
                        near: 0.0,
                        far: (hi.z - lo.z) + 10.0,
                        ..OrthographicProjection::default_3d()
                    });
                }
                cur.phase = Phase::Settling;
                cur.frames = 0;
            } else if cur.frames > LOAD_TIMEOUT {
                eprintln!("iconbake: {} never spawned a mesh", cur.subject.stem);
                bake.failed.push(cur.subject.stem);
                commands.entity(cur.root).despawn();
                return;
            }
        }
        Phase::Settling => {
            let wait = if bake.shot == 0 { SETTLE_FIRST } else { SETTLE };
            if cur.frames >= wait {
                written.0 = false;
                let path = bake.out.join(format!("{}.png", cur.subject.stem));
                commands
                    .spawn(Screenshot::image(bake.target.clone()))
                    .observe(
                        move |ev: On<ScreenshotCaptured>, mut done: ResMut<Written>| {
                            match ev.image.clone().try_into_dynamic() {
                                Ok(img) => match img.to_rgba8().save(&path) {
                                    Ok(()) => println!("iconbake: {}", path.display()),
                                    Err(e) => eprintln!("iconbake: {}: {e}", path.display()),
                                },
                                Err(e) => eprintln!("iconbake: readback: {e:?}"),
                            }
                            done.0 = true;
                        },
                    );
                cur.phase = Phase::Shooting;
                cur.frames = 0;
            }
        }
        Phase::Shooting => {
            if written.0 {
                commands.entity(cur.root).despawn();
                bake.shot += 1;
                return;
            }
        }
    }
    bake.cur = Some(cur);
}

/// The world-space box around every mesh under `root`, once they exist.
fn world_bounds(
    root: Entity,
    children: &Query<&Children>,
    bounds: &Query<(&Aabb, &GlobalTransform)>,
) -> Option<(Vec3, Vec3)> {
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    let mut any = false;
    let mut stack = vec![root];
    let mut visited = 0usize;
    while let Some(e) = stack.pop() {
        visited += 1;
        if visited > 4096 {
            break;
        }
        if let Ok((aabb, gt)) = bounds.get(e) {
            let c: Vec3 = aabb.center.into();
            let h: Vec3 = aabb.half_extents.into();
            for i in 0..8 {
                let corner = c + Vec3::new(
                    if i & 1 == 0 { -h.x } else { h.x },
                    if i & 2 == 0 { -h.y } else { h.y },
                    if i & 4 == 0 { -h.z } else { h.z },
                );
                let w = gt.transform_point(corner);
                min = min.min(w);
                max = max.max(w);
            }
            any = true;
        }
        if let Ok(kids) = children.get(e) {
            stack.extend(kids.iter());
        }
    }
    any.then_some((min, max))
}
