//! `piece_bench` — a bench, not a gate: every building shape in all four
//! tiers, drawn with the real kit (`structures::build_kit`) under the game's
//! sun, exposure, tone map and hemisphere fill, photographed headless.
//!
//! ```text
//! cargo run --release -p client --features render,native --example piece_bench -- <outdir> [filter]
//! ```
//!
//! Headless, on a box with no display:
//! ```text
//! Xvfb :99 -screen 0 1280x720x24 &
//! VK_DRIVER_FILES=/usr/share/vulkan/icd.d/lvp_icd.json DISPLAY=:99 WGPU_BACKEND=vulkan \
//!   target/release/examples/piece_bench /tmp/shots
//! ```
//!
//! `filter` keeps only the shots whose label contains it. Each tier gets a
//! plot 24 m along +x: a 2×2 base (walls, doorway, window, a half wall, and a
//! storey of floor, floor frame, roof and halves on top), a row of every
//! riser on foundations in front of it, foundation steps up to the base, and
//! the half-cell foundations and floor frame beside it. No atmosphere, so the
//! sun is dimmed by a fixed factor to stand in for the air it crosses.

use std::path::PathBuf;

use bevy::camera::Exposure;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::light::light_consts::lux;
use bevy::prelude::*;
use bevy::render::view::screenshot::{save_to_disk, Screenshot};

use client::render::structures::{
    base_transform, build_kit, edge_apron_transform, is_edge_shape, piece_local, post_owner,
    skirt_step, soft_face_positive_x, Addr, Kit, PostOwn, N_TIERS,
};
use sim_core::build::*;
use sim_core::collide::ColIndex;

/// The cell the bench's world origin sits on, so every address is positive.
const ORIGIN: u16 = 100;
/// Cells between two tiers' plots along +x.
const PLOT: u16 = 8;
/// Where the ground plane is, storey-local: the footings reach under it.
const GROUND_Y: f32 = -0.9;
/// The sun's share left after the atmosphere the bench does not draw.
const AIR: f32 = 0.8;
const SETTLE_FRAMES: u32 = 10;
const TAIL_FRAMES: u32 = 16;

#[derive(Resource)]
struct Out(PathBuf, Option<String>);

#[derive(Resource, Default)]
struct Shots {
    frame: u32,
    step: usize,
    last_shot_frame: Option<u32>,
}

#[derive(Component)]
struct Eye;

/// A piece on the bench: address offsets within a tier's plot, shape.
struct Spec {
    dx: i32,
    dz: i32,
    level: u8,
    loc: u8,
    shape: u8,
}

fn plot() -> Vec<Spec> {
    let s = |dx, dz, level, loc, shape| Spec {
        dx,
        dz,
        level,
        loc,
        shape,
    };
    let mut v = Vec::new();
    // The base: four foundations, walls round, a storey of planes on top.
    for (dx, dz) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
        v.push(s(dx, dz, 0, LOC_PLANE, SHAPE_FOUNDATION));
    }
    v.extend([
        s(0, 0, 0, LOC_EDGE_XLO, SHAPE_WALL),
        s(0, 1, 0, LOC_EDGE_XLO, SHAPE_WINDOW),
        s(2, 0, 0, LOC_EDGE_XLO, SHAPE_WINDOW),
        s(2, 1, 0, LOC_EDGE_XLO, SHAPE_WALL),
        s(0, 0, 0, LOC_EDGE_ZLO, SHAPE_DOORWAY),
        s(1, 0, 0, LOC_EDGE_ZLO, SHAPE_WALL),
        s(0, 2, 0, LOC_EDGE_ZLO, SHAPE_WALL),
        s(1, 2, 0, LOC_EDGE_ZLO, SHAPE_HALF_WALL),
        s(0, 0, 1, LOC_PLANE, SHAPE_FLOOR),
        s(1, 0, 1, LOC_PLANE, SHAPE_FLOOR_FRAME),
        s(0, 1, 1, LOC_PLANE, SHAPE_ROOF),
        s(1, 1, 1, LOC_TRI_XLO_ZLO, SHAPE_TRI_FLOOR),
        s(1, 1, 1, LOC_TRI_XHI_ZHI, SHAPE_TRI_ROOF),
        // Up the front: foundation steps to the doorway.
        s(0, -1, 0, LOC_RISER, SHAPE_FOUNDATION_STEPS),
        // Beside it: the halves, and a half frame over one.
        s(3, 0, 0, LOC_TRI_XLO_ZLO, SHAPE_TRI_FOUNDATION),
        s(3, 1, 0, LOC_TRI_XHI_ZHI, SHAPE_TRI_FOUNDATION),
        s(3, 1, 0, LOC_TRI_XLO_ZLO, SHAPE_TRI_FOUNDATION),
        s(3, 0, 1, LOC_TRI_XLO_ZLO, SHAPE_TRI_FLOOR_FRAME),
    ]);
    // The risers, each on a foundation, in a row in front of the base.
    for (i, shape) in [
        SHAPE_STAIRS,
        SHAPE_RAMP,
        SHAPE_STAIRS_L,
        SHAPE_STAIRS_U,
        SHAPE_STAIRS_SPIRAL,
        SHAPE_STAIRS_TRI_SPIRAL,
    ]
    .into_iter()
    .enumerate()
    {
        v.push(s(i as i32, -3, 0, LOC_PLANE, SHAPE_FOUNDATION));
        v.push(s(i as i32, -3, 0, LOC_RISER, shape));
    }
    v
}

fn addr(t: usize, sp: &Spec) -> Addr {
    (
        (ORIGIN as i32 + t as i32 * PLOT as i32 + sp.dx) as u16,
        (ORIGIN as i32 + sp.dz) as u16,
        sp.level,
        sp.loc,
    )
}

/// A tier's plot origin, world metres.
fn plot_at(t: usize) -> Vec3 {
    Vec3::new(t as f32 * PLOT as f32 * BUILD_CELL_M, 0.0, 0.0)
}

/// `(label, eye, look-at)`.
fn poses() -> Vec<(String, Vec3, Vec3)> {
    let mut v = Vec::new();
    let names = ["twig", "wood", "stone", "metal"];
    for (t, name) in names.iter().enumerate() {
        let o = plot_at(t);
        v.push((
            format!("{name}_base"),
            o + Vec3::new(14.0, 5.0, -9.0),
            o + Vec3::new(3.5, 1.2, 3.0),
        ));
        v.push((
            format!("{name}_risers"),
            o + Vec3::new(9.0, 6.0, -19.0),
            o + Vec3::new(9.0, 0.5, -7.5),
        ));
        v.push((
            format!("{name}_stairs"),
            o + Vec3::new(5.5, 2.6, -14.0),
            o + Vec3::new(3.0, 0.9, -7.5),
        ));
        v.push((
            format!("{name}_spirals"),
            o + Vec3::new(17.0, 3.4, -13.5),
            o + Vec3::new(13.5, 0.6, -7.5),
        ));
        v.push((
            format!("{name}_top"),
            o + Vec3::new(9.5, 7.5, -1.5),
            o + Vec3::new(3.5, 2.8, 3.5),
        ));
        v.push((
            format!("{name}_under"),
            o + Vec3::new(1.0, 1.2, 0.7),
            o + Vec3::new(4.2, 3.6, 4.2),
        ));
        v.push((
            format!("{name}_flight"),
            o + Vec3::new(-1.0, 1.9, -11.5),
            o + Vec3::new(1.5, 1.3, -7.5),
        ));
        v.push((
            format!("{name}_sun"),
            o + Vec3::new(7.5, 0.6, -0.2),
            o + Vec3::new(6.0, 2.6, 2.0),
        ));
        v.push((
            format!("{name}_wall"),
            o + Vec3::new(10.5, 1.7, -2.0),
            o + Vec3::new(6.0, 1.6, 2.5),
        ));
        v.push((
            format!("{name}_halves"),
            o + Vec3::new(13.5, 3.2, -1.5),
            o + Vec3::new(10.0, 0.6, 3.0),
        ));
    }
    v.push((
        "all_tiers".into(),
        plot_at(1) + Vec3::new(30.0, 16.0, -38.0),
        plot_at(1) + Vec3::new(18.0, 0.0, 0.0),
    ));
    v
}

fn main() -> AppExit {
    let Some(dir) = std::env::args().nth(1) else {
        eprintln!("piece_bench <outdir> [filter]");
        return AppExit::from_code(2);
    };
    let filter = std::env::args().nth(2);
    let dir = PathBuf::from(dir);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("piece_bench: {}: {e}", dir.display());
        return AppExit::from_code(2);
    }
    let assets = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: "gates — piece_bench".into(),
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
    app.insert_resource(ClearColor(Color::srgb(0.55, 0.66, 0.82)));
    app.insert_resource(Out(dir, filter));
    app.insert_resource(Shots::default());
    app.add_systems(Startup, stage);
    app.add_systems(Update, shoot);
    app.run()
}

fn stage(
    server: Res<AssetServer>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    use client::render::fill;
    let first = poses().remove(0);
    let fill_map = images.add(fill::cubemap(fill::FILL_FACE));
    commands.spawn((
        Eye,
        Camera3d::default(),
        Projection::Perspective(PerspectiveProjection {
            fov: 60.0_f32.to_radians(),
            near: 0.05,
            far: 1000.0,
            ..default()
        }),
        (
            Exposure {
                ev100: client::render::rig::DAY_EV100,
            },
            Tonemapping::TonyMcMapface,
        ),
        EnvironmentMapLight {
            diffuse_map: fill_map.clone(),
            specular_map: fill_map,
            intensity: fill::peak_lux(),
            rotation: Quat::IDENTITY,
            affects_lightmapped_mesh_diffuse: false,
        },
        Msaa::Sample4,
        Transform::from_translation(first.1).looking_at(first.2, Vec3::Y),
    ));
    let sun = client::render::rig::to_sun_at(
        client::render::rig::RIG_SUN_ELEVATION,
        client::render::rig::RIG_SUN_AZIMUTH,
    );
    commands.spawn((
        DirectionalLight {
            illuminance: lux::DIRECT_SUNLIGHT * AIR,
            shadows_enabled: true,
            ..default()
        },
        Transform::default().looking_to(-sun, Vec3::Y),
    ));
    let ground = materials.add(StandardMaterial {
        base_color: Color::linear_rgb(0.10, 0.11, 0.06),
        perceptual_roughness: 0.95,
        ..default()
    });
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(400.0, 300.0))),
        MeshMaterial3d(ground),
        Transform::from_xyz(40.0, GROUND_Y, 0.0),
    ));

    let kit = build_kit(&server, &mut meshes, &mut materials);
    let seed = 20260731;
    let haven = sim_core::terrain::haven(seed);
    let world0 = Vec3::new(
        ORIGIN as f32 * BUILD_CELL_M,
        0.0,
        ORIGIN as f32 * BUILD_CELL_M,
    );
    let specs = plot();
    let mut cols = ColIndex::new();
    for t in 0..N_TIERS {
        for sp in &specs {
            let (cx, cz, level, loc) = addr(t, sp);
            cols.add(cx, cz, level, loc, sp.shape, 0);
        }
    }
    let (step, depth) = skirt_step(-GROUND_Y + 0.4);
    let mut n = 0;
    for t in 0..N_TIERS {
        for sp in &specs {
            let a = addr(t, sp);
            let mut root = base_transform(seed, &haven, a, 0);
            root.translation.y = level_y(a.2);
            root.translation -= world0;
            spawn(
                &mut commands,
                &kit,
                &cols,
                a,
                sp.shape,
                t as u8,
                root,
                step,
                depth,
            );
            n += 1;
        }
    }
    println!("piece_bench: {n} pieces");
}

#[allow(clippy::too_many_arguments)]
fn spawn(
    commands: &mut Commands,
    kit: &Kit,
    cols: &ColIndex,
    a: Addr,
    shape: u8,
    tier: u8,
    root: Transform,
    step: usize,
    depth: f32,
) {
    let (cx, cz, level, loc) = a;
    let own = if is_edge_shape(shape) {
        post_owner(cols, cx, cz, level, loc)
    } else {
        PostOwn::default()
    };
    let soft = soft_face_positive_x(a, 0, &root);
    let transform = if matches!(shape, SHAPE_FOUNDATION | SHAPE_TRI_FOUNDATION) {
        root * Transform::from_xyz(0.0, -depth * 0.5, 0.0)
    } else {
        root * piece_local(shape)
    };
    let mat = kit.piece_material(tier, 0, a);
    let mut e = commands.spawn((
        Mesh3d(kit.piece_mesh(shape, tier, loc, own, soft, step)),
        MeshMaterial3d(mat.clone()),
        transform,
    ));
    if is_edge_shape(shape) && matches!(loc, LOC_EDGE_XLO | LOC_EDGE_ZLO) {
        if let Some(apron) = edge_apron_transform(level, 0.0) {
            e.with_child((
                Mesh3d(kit.apron_mesh(tier, own)),
                MeshMaterial3d(mat),
                apron,
            ));
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
    let list: Vec<_> = poses()
        .into_iter()
        .filter(|(l, _, _)| out.1.as_ref().is_none_or(|f| l.contains(f.as_str())))
        .collect();
    if shots.step >= list.len() {
        if shots
            .last_shot_frame
            .is_none_or(|last| shots.frame - last >= TAIL_FRAMES)
        {
            println!("piece_bench: {} shots → {}", list.len(), out.0.display());
            exit.write(AppExit::Success);
        }
        return;
    }
    // Asset loads land over the first frames; give the first pose longer.
    let settle = if shots.step == 0 {
        SETTLE_FRAMES * 4
    } else {
        SETTLE_FRAMES
    };
    let since = shots.frame - shots.last_shot_frame.unwrap_or(0);
    let (label, at, look) = &list[shots.step];
    if let Ok(mut t) = eye.single_mut() {
        *t = Transform::from_translation(*at).looking_at(*look, Vec3::Y);
    }
    if since < settle {
        return;
    }
    let path = out.0.join(format!("{label}.png"));
    println!("piece_bench: shot → {}", path.display());
    commands
        .spawn(Screenshot::primary_window())
        .observe(save_to_disk(path));
    shots.last_shot_frame = Some(shots.frame);
    shots.step += 1;
}
