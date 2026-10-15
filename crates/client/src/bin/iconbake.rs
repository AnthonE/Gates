//! `cargo run -p client --features render --bin iconbake -- <out_dir> [stem…]`
//! — photograph every item, for its inventory icon.
//!
//! Rust's icons are renders of the item's own model on a transparent ground,
//! lit from the upper left and seen from a three-quarter angle. This renders
//! every item the same way ([`SUBJECTS`]): its own model where it has one,
//! and otherwise its prop from the kit (`render::heldkit`), baked exactly as
//! the hand and the ground draw it. Then
//! `ci/finish_icons.py` turns the frames into `assets/icons/*.png`; it paints
//! a silhouette only for a stem with no render.
//!
//! Headless: no window, no winit. The camera draws into an offscreen image
//! with a transparent clear, and the frame is read back with `Screenshot`.
//! Runs on a GPU-less box through Mesa's lavapipe
//! (`apt-get install mesa-vulkan-drivers`), about a second a subject.
//!
//! Not a gate and not run by CI. Re-run it when a model or a prop changes,
//! then run the finisher:
//!
//! ```text
//! cargo run -p client --features render --bin iconbake -- ci/icons/renders
//! python3 ci/finish_icons.py
//! ```
//!
//! `--hand <out_dir> [stem…]` shoots each held row instead, the way the
//! first-person camera frames it at rest (`viewmodel::pose` under the hold,
//! no arm): the check for a new row's grip and scale.
//!
//! `--look <out_dir> <file.glb…>` photographs any files under `assets/` the
//! icon's way — a generator's rolls, before one becomes a row.

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
const SETTLE: u32 = 12;
/// A glTF that has not spawned a mesh by then is reported and skipped.
const LOAD_TIMEOUT: u32 = 900;

/// Where a subject's geometry comes from: a glTF under `assets/`, a mesh
/// `render::heldgen` builds in code (the bat), or the prop kit
/// (`render::heldkit`) for the items with no model — its own torch, revolver,
/// crossbow and strung bow included, which photograph better than the hand's.
#[derive(Clone, Copy)]
enum Src {
    Glb(&'static str),
    Gen(&'static str),
    Kit(&'static str),
}

/// One icon to photograph.
#[derive(Clone, Copy)]
struct Subject {
    /// The icon's file stem — the item's display name, normalised
    /// (`ui::icons::stem`).
    stem: &'static str,
    /// The model.
    src: Src,
    /// Model rotation before framing, degrees: yaw about Y, then pitch about
    /// X, then roll about Z (`EulerRot::YXZ`). The camera never leaves its
    /// three-quarter bearing, so this is how a subject shows its best side.
    turn: [f32; 3],
    /// Then a turn about the camera's own axis, degrees, counter-clockwise
    /// on screen: how a long thing is laid on the frame's diagonal.
    roll: f32,
    /// How much thicker than its model a long thing is drawn, across its
    /// length (model X and Z): a haft true to scale is a hairline in a 48 px
    /// cell, and the reference's tools read because they are chunky.
    girth: f32,
}

const fn s(stem: &'static str, glb: &'static str, turn: [f32; 3]) -> Subject {
    Subject {
        stem,
        src: Src::Glb(glb),
        turn,
        roll: 0.0,
        girth: 1.0,
    }
}

/// A long thing (a tool, a spear, a bow), authored standing: turned to show
/// its head's broad side, thickened by `girth`, then laid on the diagonal
/// with the head at the top right — the way Rust frames every tool.
const fn long(stem: &'static str, src: Src, girth: f32) -> Subject {
    Subject {
        stem,
        src,
        turn: [0.0, 0.0, 0.0],
        roll: -45.0,
        girth,
    }
}

/// A prop from the kit, turned.
const fn kit(stem: &'static str, turn: [f32; 3]) -> Subject {
    Subject {
        stem,
        src: Src::Kit(stem),
        turn,
        roll: 0.0,
        girth: 1.0,
    }
}

/// A model laid side on and turned `roll` on screen: a gun's framing. A
/// generated gun is authored muzzle to -X, the kit's to +X, so it is turned
/// round first and every gun in the bag points the same way.
const fn side(stem: &'static str, glb: &'static str, roll: f32) -> Subject {
    Subject {
        stem,
        src: Src::Glb(glb),
        turn: [180.0, 0.0, 0.0],
        roll,
        girth: 1.0,
    }
}

/// A long prop from the kit, on the diagonal.
const fn kit_long(stem: &'static str, roll: f32) -> Subject {
    Subject {
        stem,
        src: Src::Kit(stem),
        turn: [0.0, 0.0, 0.0],
        roll,
        girth: 1.0,
    }
}

/// Every item but the three keycards' painted cousins is photographed
/// (2026-10): the models where an item has one, the prop kit where it does
/// not. The finisher paints a silhouette only for a stem with no render.
///
/// **Long things are thickened, and that was measured.** On 2026-09-24 the
/// tools rendered true to scale went to a hairline at the cell's size and
/// the painted silhouettes beat them; laid on the diagonal and drawn
/// [`Subject::girth`] times thicker across their length they read, and
/// they read as the reference's do — the same object, lit. Framing is the
/// finisher's, from the pixels, so there is no size here.
const SUBJECTS: &[Subject] = &[
    // The resources, as the hand carries them (`WANTED.md` §12).
    s("stone", "models/held/stone.glb", [0.0, 0.0, 0.0]),
    s("metal_ore", "models/held/metal_ore.glb", [0.0, 0.0, 0.0]),
    s("sulfur_ore", "models/held/sulfur_ore.glb", [0.0, 0.0, 0.0]),
    s("wood", "models/held/wood.glb", [0.0, 0.0, 0.0]),
    s("cloth", "models/held/cloth.glb", [0.0, 0.0, 0.0]),
    s("charcoal", "models/held/charcoal.glb", [0.0, 0.0, 0.0]),
    s(
        "metal_fragments",
        "models/held/metal_fragments.glb",
        [0.0, 0.0, 0.0],
    ),
    s(
        "low_grade_fuel",
        "models/held/low_grade_fuel.glb",
        [25.0, 0.0, 0.0],
    ),
    s("junk", "models/held/junk.glb", [0.0, 0.0, 0.0]),
    s("leather", "models/held/leather.glb", [0.0, 0.0, 0.0]),
    s("sulfur", "models/held/sulfur.glb", [0.0, 0.0, 0.0]),
    s("gunpowder", "models/held/gunpowder.glb", [0.0, 0.0, 0.0]),
    s("animal_fat", "models/held/animal_fat.glb", [0.0, 0.0, 0.0]),
    s(
        "bone_fragments",
        "models/held/bone_fragments.glb",
        [0.0, 0.0, 0.0],
    ),
    s("tarp", "models/held/tarp.glb", [0.0, 0.0, 0.0]),
    s("waterskin", "models/held/waterskin.glb", [0.0, 0.0, 0.0]),
    s(
        "full_waterskin",
        "models/held/full_waterskin.glb",
        [0.0, 0.0, 0.0],
    ),
    // The small box shares this model; the finisher draws it smaller.
    s("large_box", "models/deploy/box.glb", [0.0, 0.0, 0.0]),
    s("sleeping_bag", "models/deploy/bag.glb", [-20.0, 0.0, 0.0]),
    s("workbench", "models/deploy/workbench.glb", [0.0, 0.0, 0.0]),
    // The hearth is the tool cupboard, and the render is the cupboard.
    s("hearth", "models/deploy/hearth.glb", [0.0, 0.0, 0.0]),
    // The tools and weapons with models, on the diagonal.
    long("bat", Src::Gen("bat"), 1.0),
    long(
        "stone_hatchet",
        Src::Glb("models/held/stone_hatchet.glb"),
        1.9,
    ),
    long(
        "stone_pickaxe",
        Src::Glb("models/held/stone_pickaxe.glb"),
        1.9,
    ),
    long(
        "metal_hatchet",
        Src::Glb("models/held/metal_hatchet.glb"),
        1.9,
    ),
    long(
        "metal_pickaxe",
        Src::Glb("models/held/metal_pickaxe.glb"),
        1.9,
    ),
    long("hammer", Src::Glb("models/held/hammer.glb"), 1.9),
    long(
        "wooden_spear",
        Src::Glb("models/held/wooden_spear.glb"),
        2.4,
    ),
    long("metal_spear", Src::Glb("models/held/metal_spear.glb"), 2.2),
    // Things you hold up to use.
    s(
        "building_plan",
        "models/held/building_plan.glb",
        [0.0, 0.0, 0.0],
    ),
    s(
        "satchel_charge",
        "models/held/satchel_charge.glb",
        [0.0, 0.0, 0.0],
    ),
    s("bandage", "models/held/bandage.glb", [0.0, 0.0, 0.0]),
    s("medkit", "models/held/medkit.glb", [0.0, 0.0, 0.0]),
    s("berries", "models/held/berries.glb", [0.0, 0.0, 0.0]),
    s("mushrooms", "models/held/mushrooms.glb", [0.0, 0.0, 0.0]),
    s("corn", "models/held/corn.glb", [0.0, 0.0, 0.0]),
    s("raw_meat", "models/held/raw_meat.glb", [0.0, 0.0, 0.0]),
    s(
        "cooked_meat",
        "models/held/cooked_meat.glb",
        [0.0, 0.0, 0.0],
    ),
    s("burnt_meat", "models/held/burnt_meat.glb", [0.0, 0.0, 0.0]),
    // A card held up, tipped back and turned landscape, the reference's way.
    Subject {
        stem: "green_keycard",
        src: Src::Glb("models/held/green_keycard.glb"),
        turn: [35.0, -25.0, 0.0],
        roll: 80.0,
        girth: 1.0,
    },
    Subject {
        stem: "blue_keycard",
        src: Src::Glb("models/held/blue_keycard.glb"),
        turn: [35.0, -25.0, 0.0],
        roll: 80.0,
        girth: 1.0,
    },
    Subject {
        stem: "red_keycard",
        src: Src::Glb("models/held/red_keycard.glb"),
        turn: [35.0, -25.0, 0.0],
        roll: 80.0,
        girth: 1.0,
    },
    // The deployables, as they stand in the world.
    s("furnace", "models/deploy/furnace.glb", [0.0, 0.0, 0.0]),
    s("recycler", "models/deploy/recycler.glb", [0.0, 0.0, 0.0]),
    s(
        "research_table",
        "models/deploy/research_table.glb",
        [0.0, 0.0, 0.0],
    ),
    s(
        "workbench_2",
        "models/deploy/workbench2.glb",
        [0.0, 0.0, 0.0],
    ),
    s(
        "workbench_3",
        "models/deploy/workbench3.glb",
        [0.0, 0.0, 0.0],
    ),
    s(
        "wooden_barricade",
        "models/deploy/barricade.glb",
        [0.0, 0.0, 0.0],
    ),
    // A door's face is on its X axis: a quarter turn shows it to the
    // camera (measured against 0, 30 and 60, which went edge-on).
    s("wooden_door", "models/deploy/door.glb", [90.0, 0.0, 0.0]),
    s(
        "metal_door",
        "models/deploy/door_locked.glb",
        [90.0, 0.0, 0.0],
    ),
    // The prop kit: long things on the diagonal…
    kit_long("hunting_bow", -45.0),
    kit_long("crossbow", -45.0),
    kit_long("torch", -45.0),
    kit_long("stone_spear", -45.0),
    kit_long("wooden_arrow", -45.0),
    kit_long("bone_arrow", -45.0),
    kit_long("metal_arrow", -45.0),
    kit_long("high_velocity_arrow", -45.0),
    kit_long("fire_arrow", -45.0),
    kit_long("metal_blade", -45.0),
    kit_long("medical_syringe", -45.0),
    kit_long("electric_fuse", -45.0),
    kit_long("metal_pipe", -45.0),
    kit_long("metal_spring", -30.0),
    // …guns and plates side on, nudged up at the muzzle…
    kit_long("revolver", 18.0),
    side(
        "semi_automatic_body",
        "models/held/semi_automatic_body.glb",
        18.0,
    ),
    side("smg_body", "models/held/smg_body.glb", 18.0),
    side("rifle_body", "models/held/rifle_body.glb", 18.0),
    kit_long("blueprint", 20.0),
    // …and the rest as they would sit on a table.
    kit("gears", [30.0, 0.0, 0.0]),
    kit("rope", [0.0, 0.0, 0.0]),
    kit("empty_propane_tank", [0.0, 0.0, 0.0]),
    kit("sewing_kit", [0.0, 0.0, 0.0]),
    kit("sheet_metal", [0.0, 0.0, 0.0]),
    kit("road_signs", [30.0, 0.0, 0.0]),
    kit("tech_trash", [30.0, 30.0, 0.0]),
    kit("key_lock", [30.0, 0.0, 0.0]),
    kit("code_lock", [30.0, 0.0, 0.0]),
    kit("pistol_round", [30.0, 0.0, 0.0]),
    kit("corn_seed", [0.0, 0.0, 0.0]),
    kit("hemp_seed", [0.0, 0.0, 0.0]),
    // The garments stand up in the hand; laid back for the camera.
    kit("burlap_tunic", [30.0, -20.0, 0.0]),
    kit("burlap_hood", [30.0, 0.0, 0.0]),
    kit("hide_poncho", [30.0, -20.0, 0.0]),
    kit("bone_helmet", [30.0, 0.0, 0.0]),
    kit("roadsign_vest", [30.0, -20.0, 0.0]),
    kit("planter_box", [0.0, 0.0, 0.0]),
    kit("fire_pit", [0.0, 0.0, 0.0]),
    kit("auto_turret", [30.0, 0.0, 0.0]),
    kit("metal_window_bars", [90.0, 0.0, 0.0]),
    kit("garage_door", [90.0, 0.0, 0.0]),
    kit("glass_window", [90.0, 0.0, 0.0]),
    kit("wood_shutters", [90.0, 0.0, 0.0]),
];

/// Where the camera sits relative to the subject: right, above and in front,
/// Rust's three-quarter view. Normalised at use.
const VIEW_DIR: Vec3 = Vec3::new(0.62, 0.55, 1.0);

#[derive(Resource)]
struct Bake {
    /// Shooting held rows in the first-person frame (`--hand`), not icons.
    hand: bool,
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

/// The kit's material as the studio lights it. Metal is mostly diffuse
/// here: with no environment to reflect, a metallic surface renders black but
/// for its highlights, and the game's `METAL_GAIN` is set for a hand that has
/// the island's sky in it. A third keeps the sheen and the colour both.
fn kit_material() -> StandardMaterial {
    StandardMaterial {
        metallic: 0.3 / client::render::heldkit::METAL_GAIN,
        ..client::render::heldkit::material()
    }
}

/// A frame of the `--hand` sheet, px: the game's 16:9.
const HAND_PX: (u32, u32) = (960, 540);

/// `--hand`'s subjects: every held row, in its own source, or `stems`.
fn hand_queue(only: &[&String]) -> Vec<Subject> {
    use client::ui::hold::{HeldSrc, HELD_MODELS};
    HELD_MODELS
        .iter()
        .filter(|m| only.is_empty() || only.iter().any(|o| o.as_str() == m.key))
        .map(|m| Subject {
            stem: m.key,
            src: match m.src {
                HeldSrc::Glb(path) => Src::Glb(path),
                HeldSrc::Gen(name) => Src::Gen(name),
            },
            turn: [0.0; 3],
            roll: 0.0,
            girth: 1.0,
        })
        .rev()
        .collect()
}

/// `--look`'s subjects: files, by their path under `assets/`, each shot
/// under its own file stem. Leaked: a subject's names are `'static` because
/// every other subject is a table row, and this is a one-shot process.
fn look_queue(files: &[&String]) -> Vec<Subject> {
    let root = std::env::current_dir().unwrap_or_default().join("assets");
    files
        .iter()
        .map(|f| {
            let p = std::path::Path::new(f.as_str());
            let p = p
                .canonicalize()
                .ok()
                .and_then(|abs| abs.strip_prefix(&root).ok().map(|r| r.to_path_buf()))
                .unwrap_or_else(|| p.to_path_buf());
            let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("look");
            Subject {
                stem: Box::leak(stem.to_owned().into_boxed_str()),
                src: Src::Glb(Box::leak(p.to_string_lossy().into_owned().into_boxed_str())),
                turn: [0.0; 3],
                roll: 0.0,
                girth: 1.0,
            }
        })
        .rev()
        .collect()
}

/// Where a held row's model sits in view space at rest: the rig's carry,
/// the hold, then the row's own pose about the palm.
fn hand_place(key: &str) -> Transform {
    use client::render::viewmodel::{carried, carry_of, item_pose, pose, VIEWMODEL_PALM};
    let def = client::ui::hold::HELD_MODELS
        .iter()
        .find(|m| m.key == key)
        .expect("a held row");
    carried(carry_of(Some(def)), 0.0, Quat::IDENTITY, Vec3::ZERO)
        * item_pose(None, Quat::IDENTITY)
        * pose(def, VIEWMODEL_PALM)
}

fn main() -> AppExit {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let hand = args.first().is_some_and(|a| a == "--hand");
    let look = args.first().is_some_and(|a| a == "--look");
    if hand || look {
        args.remove(0);
    }
    let Some(out) = args.first().cloned() else {
        eprintln!("usage: iconbake <out_dir> [stem…]");
        for s in SUBJECTS {
            eprintln!("  {}", s.stem);
        }
        return AppExit::from_code(2);
    };
    let only: Vec<&String> = args.iter().skip(1).collect();
    let queue: Vec<Subject> = if hand {
        hand_queue(&only)
    } else if look {
        look_queue(&only)
    } else {
        SUBJECTS
            .iter()
            .copied()
            .filter(|s| only.is_empty() || only.iter().any(|o| o.as_str() == s.stem))
            .rev()
            .collect()
    };
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
        brightness: 150.0,
        ..default()
    });
    app.insert_resource(Written::default());
    app.insert_resource(Bake {
        hand,
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
    client::render::heldkit::install(&mut images);
    let (w, h) = if bake.hand {
        HAND_PX
    } else {
        (FRAME_PX, FRAME_PX)
    };
    let target = images.add(Image::new_target_texture(
        w,
        h,
        TextureFormat::Rgba8UnormSrgb,
        None,
    ));
    bake.target = target.clone();
    if bake.hand {
        // The eye: at the origin looking down −Z, the game's lens, a dull
        // sky behind so a dark item still has an edge.
        commands.spawn((
            IconCam,
            Camera3d::default(),
            Camera {
                clear_color: ClearColorConfig::Custom(Color::srgb(0.52, 0.58, 0.64)),
                ..default()
            },
            RenderTarget::Image(target.into()),
            Projection::Perspective(PerspectiveProjection {
                fov: client::render::rig::FOV_DEG.to_radians(),
                near: 0.01,
                ..default()
            }),
            Tonemapping::AcesFitted,
            Msaa::Sample4,
            Transform::IDENTITY,
        ));
    } else {
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
            Transform::from_translation(VIEW_DIR.normalize() * 10.0)
                .looking_at(Vec3::ZERO, Vec3::Y),
        ));
    }
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
            illuminance: 1_200.0,
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
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
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
        let roll = Quat::from_axis_angle(VIEW_DIR.normalize(), subject.roll.to_radians());
        let place = Transform::from_rotation(roll * turn).with_scale(Vec3::new(
            subject.girth,
            1.0,
            subject.girth,
        ));
        let place = if bake.hand {
            hand_place(subject.stem)
        } else {
            place
        };
        let root = match subject.src {
            // The hand loads a row's file the way `viewmodel` does: its one
            // primitive and its one material.
            Src::Glb(glb) if bake.hand => commands
                .spawn((
                    Mesh3d(
                        assets.load(
                            GltfAssetLabel::Primitive {
                                mesh: 0,
                                primitive: 0,
                            }
                            .from_asset(glb),
                        ),
                    ),
                    MeshMaterial3d::<StandardMaterial>(
                        assets.load(
                            GltfAssetLabel::Material {
                                index: 0,
                                is_scale_inverted: false,
                            }
                            .from_asset(glb),
                        ),
                    ),
                    place,
                ))
                .id(),
            Src::Glb(glb) => commands
                .spawn((
                    SceneRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(glb))),
                    place,
                ))
                .id(),
            Src::Gen(name) => commands
                .spawn((
                    Mesh3d(meshes.add(client::render::heldgen::mesh(name))),
                    MeshMaterial3d(mats.add(client::render::heldgen::material(name))),
                    place,
                ))
                .id(),
            Src::Kit(name) => commands
                .spawn((
                    Mesh3d(
                        meshes.add(
                            client::render::heldkit::baked(name)
                                .unwrap_or_else(|| panic!("no kit prop {name}")),
                        ),
                    ),
                    MeshMaterial3d(mats.add(kit_material())),
                    place,
                ))
                .id(),
        };
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
            if bake.hand && world_bounds(cur.root, &children, &bounds).is_some() {
                cur.phase = Phase::Settling;
                cur.frames = 0;
            } else if let Some((min, max)) = world_bounds(cur.root, &children, &bounds) {
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
