//! `cargo run -p client --features render --bin modelview -- <file.glb> [more.glb]`
//! — a standalone rig bench. No shard, no netcode, no world.
//!
//! **Why this is a separate binary and not a flag on `gates`.** Judging a
//! character asset means answering four questions, and the game answers none
//! of them: *is it the right height*, *does its origin sit on its feet*, *do
//! the clips this game asks for exist under the names it asks for*, and *does
//! the mesh sit inside the capsule the sim shoots at*. In the game those are
//! buried under a connect, a terrain stream and a body somebody else is
//! driving. Here they are the whole screen.
//!
//! **It is a bench, not a gate** (`CLAUDE.md`: there is no visual gate and
//! none is to be built). It prints measurements and draws the sim's own
//! volumes beside the mesh so a person can look. Nothing here asserts, nothing
//! here is scored, and no CI step runs it. The arithmetic that IS worth gating
//! about a frame stays in a Rust test — `crates/client/tests/tree.rs`'s shape.
//!
//! **It links no client code on purpose.** `client::render` is the game's
//! renderer, it owns a connect and a `Screen` state machine, and a bench that
//! imports it inherits both. This file is Bevy and nothing else, which is what
//! makes it cheap: the render feature's Bevy is already compiled, so this
//! costs a link rather than a build.
//!
//! ```text
//! modelview <file.glb>...        up to 4; laid out 1.5 m apart on X
//!   --shot <dir>                 headless: settle, shoot 4 bearings, exit
//!   --clip <name>                start on this clip instead of the first
//!   --rot-x <deg>                pre-rotate about X (a Z-up export is 90)
//!   --scale <k>                  uniform scale applied before anything
//!   --eye                        camera at the first-person eye, in the head
//!   --hide A,B                   collapse these bones to nothing
//!   --per-clip                   with --shot: one frame per clip, named
//!   --no-fix                     do not auto-correct a Z-up export
//! ```
//!
//! Keys: `[` `]` clip · `space` pause · `r` re-fix rotation · `c` capsule ·
//! `g` grid · `t` bind pose · drag / arrows orbit · wheel zoom · `p` shot.

use std::f32::consts::{FRAC_PI_2, TAU};
use std::path::PathBuf;

use bevy::camera::primitives::Aabb;
use bevy::gltf::Gltf;
use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit};
use bevy::mesh::skinning::{SkinnedMesh, SkinnedMeshInverseBindposes};
use bevy::prelude::*;
use bevy::render::view::screenshot::{save_to_disk, Screenshot};

/// What the sim collides and shoots with — `render::anim::ANIM_BODY_H_M` and
/// `bodies.rs`'s retired `Capsule3d::new(0.4, 1.0)`. Restated rather than
/// imported because this binary deliberately links no client code; the two
/// numbers agreeing is the thing the bench is for, so a copy that drifts is
/// visible on screen rather than hidden behind a shared constant.
const BODY_H_M: f32 = 1.8;
const BODY_R_M: f32 = 0.4;
/// First-person eye — `render::mod::EYE_HEIGHT`. Drawn because a head that
/// sits above it is a head the camera is inside of.
const EYE_H_M: f32 = 1.6;

/// The clip names `render::anim::Clip` actually resolves. A model that answers
/// all three drops into the game; one that answers none needs its clips
/// renamed at import (`ci/import_char.py --rename OLD=NEW`).
///
/// Three and not five, because two of the enum's variants are aliases: `Sleep`
/// plays the idle (a sleeper is a person standing still) and `Sprint` plays
/// the jog faster (there is no sprint clip in the shipped rig). Listing the
/// alias targets would report a model as complete when it is missing the two
/// clips [`ALIASED`] names — so the bench prints both lists and neither
/// pretends to be the other.
const WANTED: [&str; 3] = ["Idle_Loop", "Walk_Loop", "Jog_Fwd_Loop"];
/// Clips the game has a state for and no animation of: it substitutes one of
/// [`WANTED`] instead. A rig that supplies these is a rig that improves the
/// game without a line of code changing.
const ALIASED: [&str; 2] = ["Sprint_Loop", "Sleep_Loop"];

/// How many subjects fit on the bench. Wall 4's habit on a path a person
/// drives: a cap and a stated policy, which is *refuse the extras and say so*.
const SUBJECT_CAP: usize = 4;
/// Metres between subjects, left to right.
const SUBJECT_PITCH_M: f32 = 1.5;

fn main() -> AppExit {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut paths: Vec<PathBuf> = Vec::new();
    let mut opts = Opts::default();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                return AppExit::Success;
            }
            "--shot" => match it.next() {
                Some(d) => opts.shot = Some(PathBuf::from(d)),
                None => return bad("--shot wants a directory"),
            },
            "--clip" => match it.next() {
                Some(c) => opts.clip = Some(c.clone()),
                None => return bad("--clip wants a name"),
            },
            "--rot-x" => match it.next().and_then(|v| v.parse::<f32>().ok()) {
                Some(d) => opts.rot_x_deg = Some(d),
                None => return bad("--rot-x wants degrees"),
            },
            "--scale" => match it.next().and_then(|v| v.parse::<f32>().ok()) {
                Some(k) if k > 0.0 => opts.scale = k,
                _ => return bad("--scale wants a positive number"),
            },
            "--no-fix" => opts.auto_fix = false,
            "--per-clip" => opts.per_clip = true,
            "--eye" => opts.eye = true,
            "--hide" => match it.next() {
                Some(v) => opts.hide.extend(v.split(',').map(|s| s.trim().to_string())),
                None => return bad("--hide wants a comma-separated bone list"),
            },
            other if other.starts_with('-') => return bad(&format!("unknown flag {other}")),
            other => paths.push(PathBuf::from(other)),
        }
    }
    if paths.is_empty() {
        return bad("nothing to look at");
    }
    if paths.len() > SUBJECT_CAP {
        eprintln!(
            "modelview: {} files given, showing the first {SUBJECT_CAP}",
            paths.len()
        );
        paths.truncate(SUBJECT_CAP);
    }

    // **The asset root has to contain the file, and a glTF's own paths are
    // relative to it.** `gates.rs` points Bevy at `<cwd>/assets`; a bench
    // takes a path from a person, which may be anywhere. So the root is the
    // deepest directory common to every subject and each one loads by its
    // name under it — which is also what makes `mannequin.gltf` work, since
    // its buffer `uri` is a sibling filename the loader resolves against the
    // root and not against the process.
    let (root, names) = match split_root(&paths) {
        Ok(v) => v,
        Err(e) => return bad(&e),
    };

    let mut app = App::new();
    let mut assets = AssetPlugin {
        file_path: root.to_string_lossy().into_owned(),
        ..default()
    };
    // Never a watcher: a bench that reloads under a shot is a shot nobody can
    // reproduce (`client/Cargo.toml`'s `hot` feature says the same thing).
    assets.watch_for_changes_override = Some(false);
    app.add_plugins(DefaultPlugins.set(assets).set(WindowPlugin {
        primary_window: Some(Window {
            title: "gates — modelview".into(),
            resolution: (1280u32, 720u32).into(),
            ..default()
        }),
        ..default()
    }));
    app.insert_resource(ClearColor(Color::srgb(0.16, 0.17, 0.19)));
    app.insert_resource(opts);
    app.insert_resource(Bench {
        names,
        subjects: Vec::new(),
        clips: Vec::new(),
        current: 0,
        paused: false,
        bind_pose: false,
        show_capsule: true,
        show_grid: true,
    });
    app.insert_resource(Orbit::default());
    app.insert_resource(Shots::default());
    app.add_systems(Startup, (stage, load));
    app.add_systems(
        Update,
        (
            build,
            bind,
            report,
            drive,
            orbit,
            keys,
            hide_bones,
            draw,
            hud,
            snap,
            shoot.run_if(|o: Res<Opts>| o.shot.is_some()),
        )
            .chain(),
    );
    app.run()
}

const USAGE: &str = "\
modelview <file.glb>... [--shot <dir>] [--per-clip] [--clip <name>]
          [--rot-x <deg>] [--scale <k>] [--eye] [--hide A,B] [--no-fix]

A rig bench: loads glTF/GLB files side by side against the sim's own 1.8 m
capsule, lists their clips, and plays them. Not a gate — a thing to look at.";

fn bad(why: &str) -> AppExit {
    eprintln!("modelview: {why}\n\n{USAGE}");
    AppExit::from_code(2)
}

/// Split a list of paths into one asset root and the names under it. Bevy's
/// asset server takes a single root, so a bench comparing two files in
/// different directories has to find their common ancestor.
fn split_root(paths: &[PathBuf]) -> Result<(PathBuf, Vec<String>), String> {
    let mut abs = Vec::new();
    for p in paths {
        let a = p
            .canonicalize()
            .map_err(|e| format!("{}: {e}", p.display()))?;
        if !a.is_file() {
            return Err(format!("{}: not a file", p.display()));
        }
        abs.push(a);
    }
    let mut root = abs[0].parent().unwrap_or(&abs[0]).to_path_buf();
    for a in &abs[1..] {
        let mut dir = a.parent().unwrap_or(a).to_path_buf();
        // Walk both up until one contains the other. Bounded — a path has
        // finitely many components and a `loop` here would hang on a
        // pathological input.
        for _ in 0..64 {
            if a.starts_with(&root) && abs[0].starts_with(&root) {
                break;
            }
            if dir.starts_with(&root) {
                break;
            }
            match root.parent() {
                Some(p) => root = p.to_path_buf(),
                None => break,
            }
            dir = dir.parent().map(|p| p.to_path_buf()).unwrap_or(dir);
        }
    }
    let names = abs
        .iter()
        .map(|a| {
            a.strip_prefix(&root)
                .map(|r| r.to_string_lossy().into_owned())
                .unwrap_or_else(|_| a.to_string_lossy().into_owned())
        })
        .collect();
    Ok((root, names))
}

/// The flags, as given. A resource so the systems below read one copy.
#[derive(Resource)]
struct Opts {
    shot: Option<PathBuf>,
    clip: Option<String>,
    rot_x_deg: Option<f32>,
    scale: f32,
    /// With `--shot`: one frame per CLIP from a fixed three-quarter bearing,
    /// named after the clip, instead of four bearings of one clip.
    per_clip: bool,
    /// Bones to collapse to zero scale, by name.
    ///
    /// **The trick, and its limit.** A character with ONE mesh and ONE
    /// material cannot hide a limb with `Visibility` — that is per entity, and
    /// the whole body is one entity. What it can do is scale a JOINT to zero,
    /// which collapses every vertex weighted to it onto that joint's origin.
    /// The limit is the hierarchy: scale collapses children too, so hiding the
    /// torso also hides the arms hanging off it. Practically this hides the
    /// head (`--hide Head`) and the legs (`--hide LeftUpLeg,RightUpLeg`), and
    /// cannot hide the torso of a body whose arms you are keeping.
    hide: Vec<String>,
    /// Put the camera where the first-person eye is — inside the head, at
    /// `EYE_H_M`, looking forward — instead of orbiting the body. What the
    /// player would see of their own body, which is the question "can we have
    /// arms in first person" actually asks.
    eye: bool,
    /// Correct an export whose up axis is Z. Meshy's merged-animation exports
    /// arrive that way — the mesh bounds run along -Z and the character lies
    /// on its back — and the fix is one rotation on the root, which carries
    /// the skeleton and every clip with it. Off with `--no-fix` so the raw
    /// file can be seen as it is.
    auto_fix: bool,
}

impl Default for Opts {
    fn default() -> Self {
        Self {
            shot: None,
            clip: None,
            rot_x_deg: None,
            scale: 1.0,
            hide: Vec::new(),
            eye: false,
            per_clip: false,
            auto_fix: true,
        }
    }
}

/// One thing on the bench.
struct Subject {
    name: String,
    gltf: Handle<Gltf>,
    root: Option<Entity>,
    /// World-space bounds, once the scene has spawned meshes with an `Aabb`.
    measured: bool,
}

#[derive(Resource)]
struct Bench {
    names: Vec<String>,
    subjects: Vec<Subject>,
    /// Every clip name any subject carries, in file order. The graph node for
    /// each is per-subject, so this is names only.
    clips: Vec<String>,
    current: usize,
    paused: bool,
    bind_pose: bool,
    show_capsule: bool,
    show_grid: bool,
}

/// Marks the entity a subject's scene hangs under, so the rotation fix and
/// the scale are one transform a key can rewrite.
#[derive(Component)]
struct SubjectRoot {
    /// Extra rotation about X, radians. Rewritten by `r`.
    rot_x: f32,
    /// Whether the Z-up correction has already been offered. Once, ever: a
    /// measurement that changes the thing it measures has to stop.
    fixed: bool,
}

/// The graph a subject's players share, and where each clip landed in it.
#[derive(Component)]
struct SubjectGraph {
    handle: Handle<AnimationGraph>,
    nodes: Vec<(String, AnimationNodeIndex)>,
}

/// Which subject an `AnimationPlayer` belongs to — the player lands on a
/// descendant of the spawned scene, so the link is walked once (`anim.rs`).
#[derive(Component)]
struct PlayerOf(Entity);

/// What that player is playing, so `drive` writes on a transition rather than
/// restarting the clip every frame (`anim.rs`'s `Playing` — the same defect
/// looks exactly like "the animation did not load").
#[derive(Component, Default)]
struct Playing(Option<String>);

#[derive(Component)]
struct BenchCam;

#[derive(Component)]
struct HudText;

#[derive(Resource)]
struct Orbit {
    yaw: f32,
    pitch: f32,
    dist: f32,
    height: f32,
}

impl Default for Orbit {
    fn default() -> Self {
        Self {
            yaw: 0.0,
            pitch: -0.12,
            dist: 4.2,
            height: 0.95,
        }
    }
}

/// Camera, sun, floor, overlay. No atmosphere and no tonemapping curve worth
/// arguing about: this bench answers questions about geometry, and a neutral
/// grey room is what makes a silhouette readable.
fn stage(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        BenchCam,
        Camera3d::default(),
        Projection::Perspective(PerspectiveProjection {
            fov: 55.0_f32.to_radians(),
            near: 0.05,
            far: 200.0,
            ..default()
        }),
        Transform::from_xyz(0.0, 1.0, 4.0).looking_at(Vec3::new(0.0, 0.9, 0.0), Vec3::Y),
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: 12_000.0,
            shadows_enabled: true,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(EulerRot::YXZ, -0.7, -0.9, 0.0)),
    ));
    // A second, dim, opposite light. Not physics — a bench: an unlit back side
    // reads as a hole in the mesh and sends you hunting for a normals bug.
    commands.spawn((
        DirectionalLight {
            illuminance: 2_500.0,
            shadows_enabled: false,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(EulerRot::YXZ, 2.4, -0.5, 0.0)),
    ));
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(40.0, 40.0))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.30, 0.31, 0.33),
            perceptual_roughness: 0.95,
            ..default()
        })),
        Transform::from_xyz(0.0, 0.0, 0.0),
    ));
    commands.spawn((
        HudText,
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(10.0),
            left: Val::Px(12.0),
            ..default()
        },
        Text::new("modelview: loading…"),
        TextFont {
            font_size: 13.0,
            ..default()
        },
        TextColor(Color::srgb(0.92, 0.93, 0.95)),
    ));
}

fn load(mut commands: Commands, assets: Res<AssetServer>, mut bench: ResMut<Bench>) {
    let names = bench.names.clone();
    for name in names {
        let gltf = assets.load::<Gltf>(name.clone());
        bench.subjects.push(Subject {
            name,
            gltf,
            root: None,
            measured: false,
        });
    }
    // Silences an unused warning in the branch where nothing loads, and keeps
    // `commands` in the signature for symmetry with the rest of the file.
    let _ = &mut commands;
}

/// Spawn each subject once its glTF is in, building one graph over EVERY clip
/// the file carries.
///
/// **Clips are resolved by name, never by index** — `anim.rs`'s rule, and the
/// bench exists partly to check those names, so reading them positionally
/// would defeat the point. `named_animations` is a map with no order, so the
/// list is sorted: two runs of this bench must name the clips in the same
/// order or a note taken from one run does not describe the other.
fn build(
    mut commands: Commands,
    mut bench: ResMut<Bench>,
    opts: Res<Opts>,
    gltfs: Res<Assets<Gltf>>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
) {
    for i in 0..bench.subjects.len() {
        if bench.subjects[i].root.is_some() {
            continue;
        }
        let Some(gltf) = gltfs.get(&bench.subjects[i].gltf) else {
            continue;
        };
        let Some(scene) = gltf
            .default_scene
            .clone()
            .or_else(|| gltf.scenes.first().cloned())
        else {
            error!("{}: no scene in the file", bench.subjects[i].name);
            // Marked spawned with a placeholder so this does not retry every
            // frame forever.
            bench.subjects[i].root = Some(Entity::PLACEHOLDER);
            continue;
        };

        let mut names: Vec<String> = gltf
            .named_animations
            .keys()
            .map(|k| k.to_string())
            .collect();
        names.sort();

        let mut graph = AnimationGraph::new();
        let root = graph.root;
        let mut nodes = Vec::new();
        for n in &names {
            if let Some(h) = gltf.named_animations.get(n.as_str()) {
                nodes.push((n.clone(), graph.add_clip(h.clone(), 1.0, root)));
            }
        }
        let handle = graphs.add(graph);

        let x = (i as f32 - (bench.names.len() as f32 - 1.0) * 0.5) * SUBJECT_PITCH_M;
        let rot_x = opts.rot_x_deg.map(|d| d.to_radians()).unwrap_or(0.0);
        let entity = commands
            .spawn((
                SubjectRoot {
                    rot_x,
                    // An explicit `--rot-x` is a person's answer; the bench
                    // does not then offer its own.
                    fixed: opts.rot_x_deg.is_some(),
                },
                SubjectGraph { handle, nodes },
                SceneRoot(scene),
                Transform::from_xyz(x, 0.0, 0.0)
                    .with_rotation(Quat::from_rotation_x(rot_x))
                    .with_scale(Vec3::splat(opts.scale)),
            ))
            .id();
        bench.subjects[i].root = Some(entity);

        println!("── {} ──", bench.subjects[i].name);
        println!("   clips: {}", names.len());
        for n in &names {
            println!("     {n}");
        }
        let missing: Vec<&str> = WANTED
            .iter()
            .copied()
            .filter(|w| !names.iter().any(|n| n == w))
            .collect();
        if missing.is_empty() {
            println!("   render::anim: every clip it resolves is here");
        } else {
            println!(
                "   render::anim: {} of {} names missing → {:?}",
                missing.len(),
                WANTED.len(),
                missing
            );
        }
        let absent: Vec<&str> = ALIASED
            .iter()
            .copied()
            .filter(|w| !names.iter().any(|n| n == w))
            .collect();
        if !absent.is_empty() {
            println!(
                "   substituted: {absent:?} — the game has a state for each and \
                 plays another clip for it"
            );
        }
        for n in names {
            if !bench.clips.contains(&n) {
                bench.clips.push(n);
            }
        }
        // Open on something worth looking at. Alphabetical order hands you
        // `A_TPose`, which is the one pose that tells you nothing.
        if let Some(k) = bench
            .clips
            .iter()
            .position(|n| Some(n.as_str()) == opts.clip.as_deref())
            .or_else(|| {
                bench
                    .clips
                    .iter()
                    .position(|n| WANTED.contains(&n.as_str()))
            })
        {
            bench.current = k;
        }
    }
}

/// Attach the graph to every newly spawned player and find its subject.
fn bind(
    mut commands: Commands,
    parents: Query<&ChildOf>,
    graphs: Query<&SubjectGraph>,
    added: Query<Entity, Added<AnimationPlayer>>,
) {
    for player in &added {
        // Bounded walk up to the subject root — `anim::bind`'s shape, and for
        // its reason: a malformed hierarchy must not hang a frame.
        let mut at = player;
        let mut found = None;
        for _ in 0..16 {
            if graphs.get(at).is_ok() {
                found = Some(at);
                break;
            }
            match parents.get(at) {
                Ok(p) => at = p.0,
                Err(_) => break,
            }
        }
        let Some(subject) = found else { continue };
        let Ok(graph) = graphs.get(subject) else {
            continue;
        };
        commands.entity(player).insert((
            AnimationGraphHandle(graph.handle.clone()),
            AnimationTransitions::new(),
            PlayerOf(subject),
            Playing::default(),
        ));
    }
}

/// Measure each subject once its meshes have bounds, and say what it is.
///
/// **The numbers are the point of the bench.** A character asset is right or
/// wrong about three things a picture will not tell you reliably: how tall it
/// is, whether its origin is on its feet, and whether it fits the volume the
/// server shoots at. All three are arithmetic over the spawned scene.
fn report(
    mut bench: ResMut<Bench>,
    opts: Res<Opts>,
    mut roots: Query<(&mut SubjectRoot, &mut Transform, &GlobalTransform)>,
    children: Query<&Children>,
    bounds: Query<(&Aabb, &GlobalTransform, Option<&SkinnedMesh>)>,
    xforms: Query<&GlobalTransform>,
    bindposes: Res<Assets<SkinnedMeshInverseBindposes>>,
) {
    for i in 0..bench.subjects.len() {
        if bench.subjects[i].measured {
            continue;
        }
        let Some(root) = bench.subjects[i].root else {
            continue;
        };
        if root == Entity::PLACEHOLDER {
            bench.subjects[i].measured = true;
            continue;
        }
        let Ok((mut sub, mut tf, root_tf)) = roots.get_mut(root) else {
            continue;
        };
        let Some((min, max)) = world_bounds(root, root_tf, &children, &bounds, &xforms, &bindposes)
        else {
            // The scene spawns asynchronously; nothing to measure yet.
            continue;
        };
        let origin = root_tf.translation();
        let h = max.y - min.y;
        let w = max.x - min.x;
        let d = max.z - min.z;

        // **A Z-up export, stood up before it is described.** Meshy's merged
        // animation files arrive with the height along -Z — the character
        // lies on its back — and every number below would then describe a
        // fallen body. The correction is one rotation on the root, which
        // carries the skeleton and every clip with it, and it is ANNOUNCED:
        // a person choosing an import needs to know the file is wrong, not
        // just see it standing. `--no-fix` shows it as it is.
        if !sub.fixed && d > h * 1.5 {
            sub.fixed = true;
            if opts.auto_fix {
                sub.rot_x = FRAC_PI_2;
                tf.rotation = Quat::from_rotation_x(sub.rot_x);
                println!("── {} ──", bench.subjects[i].name);
                println!(
                    "   ⚠ up axis is Z ({d:.2} m deep against {h:.2} m tall) — stood up \
                     with a 90° X rotation. The fix belongs in the IMPORT, not here."
                );
                // Measured next frame, against the corrected transform: the
                // propagation has not run yet, so every number is still the
                // fallen one.
                continue;
            }
            println!("── {} ──", bench.subjects[i].name);
            println!("   ⚠ up axis looks like Z — left alone (--no-fix); press `r` to stand it up");
        }

        println!("── {} ──", bench.subjects[i].name);
        println!("   size    {h:.3} m tall, {w:.3} wide, {d:.3} deep   (sim body {BODY_H_M} m)");
        println!(
            "   feet    y {:.4} → {:.4}   (0 is the ground; the wire's y IS the feet)",
            min.y - origin.y,
            max.y - origin.y
        );
        println!(
            "   scale   {:.4} would put it at {BODY_H_M} m",
            BODY_H_M / h
        );
        // **This is the BIND pose and nothing else.** Bevy computes an
        // `Aabb` from the mesh as authored; it does not re-fit one to the
        // skinned result, so a T-pose rig reports its arm span as its width
        // forever. The height and the origin are trustworthy; the radius is a
        // hint, and on a T-pose it is a hint about the rig rather than the
        // body. Said out loud because a 2.4x number that means nothing is
        // worse than no number at all.
        let r = (w.max(d)) * 0.5;
        println!(
            "   radius  {r:.3} m in the BIND pose against the capsule's {BODY_R_M} — {}",
            if r <= BODY_R_M {
                "inside".to_string()
            } else {
                format!(
                    "{:.2}x wider (a T-pose arm span, if it is one)",
                    r / BODY_R_M
                )
            }
        );
        if sub.rot_x != 0.0 {
            println!("   rot-x   {:.0}° applied", sub.rot_x.to_degrees());
        }
        bench.subjects[i].measured = true;
    }
}

/// Union the world-space `Aabb` of every mesh under `root`.
///
/// **A skinned mesh does not live where its node says it does, and believing
/// the node is how this bench first reported a 1.8 m character as 18 mm.** A
/// glTF skin's vertices are authored in the space its inverse bind matrices
/// were computed in — the spec says a skinned mesh node's own transform is to
/// be ignored, and Bevy obeys it: the shader replaces the model matrix with
/// the blended joint matrix. The exporter here hangs the mesh under an
/// armature carrying `scale 0.01` (its joint translations are in centimetres),
/// so reading the node's `GlobalTransform` shrinks the measurement by exactly
/// that hundred while the picture on screen is the right size. **Which is the
/// tell**: the frame and the number disagreed, and the number was wrong.
///
/// **And the subject root is not the answer either** — that was this
/// function's second wrong version, and the import is what exposed it. Once
/// `ci/import_char.py` writes the stand-up rotation onto the glTF's OWN root
/// node, that node sits between the bench's spawn root and the skin, so
/// measuring against the bench's root misses it: the file was corrected, the
/// picture was right, and the bench reported it still lying down and rotated
/// it a second time. A measurement that is one node short is not a small
/// error, it is an error that fights the fix.
///
/// The exact transform is the one the GPU uses: `JointWorld_j · IBM_j`, which
/// at bind pose is the same matrix for every `j` by construction. Joint 0 and
/// its inverse bindpose is therefore the whole answer, and it needs no
/// assumption at all about where the file put its roots. Unskinned meshes keep
/// the node's own transform, which for them is correct.
fn world_bounds(
    root: Entity,
    root_gt: &GlobalTransform,
    children: &Query<&Children>,
    bounds: &Query<(&Aabb, &GlobalTransform, Option<&SkinnedMesh>)>,
    xforms: &Query<&GlobalTransform>,
    bindposes: &Assets<SkinnedMeshInverseBindposes>,
) -> Option<(Vec3, Vec3)> {
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    let mut any = false;
    // Bounded breadth-first walk — `anim::reshade`'s cap, same reason.
    let mut stack = vec![root];
    let mut visited = 0usize;
    while let Some(e) = stack.pop() {
        visited += 1;
        if visited > 4096 {
            break;
        }
        if let Ok((aabb, node_gt, skin)) = bounds.get(e) {
            // `JointWorld_0 · IBM_0` for a skin, the node's own transform
            // otherwise. The fallback when a skin's joints or bindposes are
            // not loaded yet is the bench root, which is wrong by whatever
            // the file's own root carries — so it is only ever a stopgap for
            // the frame before the asset lands.
            let skinned: Option<Mat4> = skin.and_then(|s| {
                let ibp = bindposes.get(&s.inverse_bindposes)?;
                let j0 = s.joints.first()?;
                let jgt = xforms.get(*j0).ok()?;
                Some(jgt.to_matrix() * *ibp.first()?)
            });
            let node = if skin.is_some() { root_gt } else { node_gt };
            let c: Vec3 = aabb.center.into();
            let h: Vec3 = aabb.half_extents.into();
            for i in 0..8 {
                let corner = c + Vec3::new(
                    if i & 1 == 0 { -h.x } else { h.x },
                    if i & 2 == 0 { -h.y } else { h.y },
                    if i & 4 == 0 { -h.z } else { h.z },
                );
                let p = match skinned {
                    Some(m) => m.transform_point3(corner),
                    None => node.transform_point(corner),
                };
                min = min.min(p);
                max = max.max(p);
            }
            any = true;
        }
        if let Ok(kids) = children.get(e) {
            stack.extend(kids.iter());
        }
    }
    any.then_some((min, max))
}

/// Cross-fade every player to the bench's current clip, and honour pause.
fn drive(
    bench: Res<Bench>,
    opts: Res<Opts>,
    graphs: Query<&SubjectGraph>,
    mut players: Query<(
        &PlayerOf,
        &mut Playing,
        &mut AnimationPlayer,
        &mut AnimationTransitions,
    )>,
) {
    let want = bench.clips.get(bench.current).cloned();
    for (owner, mut playing, mut player, mut transitions) in &mut players {
        if bench.bind_pose {
            if playing.0.is_some() {
                player.stop_all();
                playing.0 = None;
            }
            continue;
        }
        let Ok(graph) = graphs.get(owner.0) else {
            continue;
        };
        // A subject that does not carry the selected clip keeps whatever it
        // had. The alternative — falling back to its first clip — makes two
        // models look like they agree when they do not.
        let want = want
            .clone()
            .or_else(|| opts.clip.clone())
            .filter(|w| graph.nodes.iter().any(|(n, _)| n == w));
        let Some(want) = want else { continue };
        let Some((_, node)) = graph.nodes.iter().find(|(n, _)| *n == want) else {
            continue;
        };
        if playing.0.as_ref() != Some(&want) {
            playing.0 = Some(want);
            transitions
                .play(&mut player, *node, std::time::Duration::from_secs_f32(0.15))
                .repeat();
        }
        // Pause is a speed of zero rather than `pause()`, so scrubbing back to
        // motion does not restart the clip.
        player.adjust_speeds(if bench.paused { 0.0 } else { 1.0 });
    }
}

/// Collapse the named bones. Runs until each one has been found once, then
/// costs a `Vec::is_empty` — the scene spawns asynchronously, so a one-shot
/// pass at spawn time would silently find nothing.
fn hide_bones(
    opts: Res<Opts>,
    mut done: Local<bool>,
    mut named: Query<(&Name, &mut Transform, Option<&mut Visibility>)>,
) {
    if *done || opts.hide.is_empty() {
        return;
    }
    let mut hit = 0;
    for (name, mut t, vis) in &mut named {
        if !opts.hide.iter().any(|h| h == name.as_str()) {
            continue;
        }
        // **Both mechanisms, unconditionally, because only one of them works
        // per target and choosing between them needs a fact the entity does
        // not carry.** A MESH node hides by `Visibility` — it has to, since a
        // skinned mesh's node transform is ignored by the spec and scaling it
        // to zero changes nothing. A BONE hides by scale, which collapses the
        // vertices weighted to it. The obvious dispatch — `Has<Mesh3d>` — is
        // WRONG and was written that way first: Bevy names the glTF *node* and
        // hangs the `Mesh3d` on a primitive CHILD of it, so the named entity
        // has no mesh, every target took the bone branch, and the half meant
        // to disappear stayed on screen while the tool reported success.
        // Doing both is free: visibility on a bone reaches no mesh, and scale
        // on a skinned mesh node is ignored.
        if let Some(mut v) = vis {
            *v = Visibility::Hidden;
        }
        t.scale = Vec3::splat(1e-4);
        hit += 1;
    }
    if hit > 0 {
        *done = true;
        println!("   hidden: {} of {:?}", hit, opts.hide);
    }
}

/// The camera: an orbit, or the first-person eye under `--eye`.
///
/// Eight parameters, which clippy counts and a Bevy system does not get to
/// bundle away without a `SystemParam` struct that would exist only to satisfy
/// the count. Allowed by name rather than restructured — this is a bench, and
/// the arguments are each one input a camera genuinely has.
#[allow(clippy::too_many_arguments)]
fn orbit(
    mut cam: Query<&mut Transform, With<BenchCam>>,
    opts: Res<Opts>,
    mut o: ResMut<Orbit>,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    time: Res<Time>,
) {
    let dt = time.delta_secs();
    if buttons.pressed(MouseButton::Left) {
        o.yaw -= motion.delta.x * 0.006;
        o.pitch = (o.pitch - motion.delta.y * 0.006).clamp(-1.4, 1.4);
    }
    let k = 1.4 * dt;
    if keys.pressed(KeyCode::ArrowLeft) {
        o.yaw -= k;
    }
    if keys.pressed(KeyCode::ArrowRight) {
        o.yaw += k;
    }
    if keys.pressed(KeyCode::ArrowUp) {
        o.pitch = (o.pitch + k).min(1.4);
    }
    if keys.pressed(KeyCode::ArrowDown) {
        o.pitch = (o.pitch - k).max(-1.4);
    }
    if keys.pressed(KeyCode::KeyW) {
        o.height += dt;
    }
    if keys.pressed(KeyCode::KeyS) {
        o.height -= dt;
    }
    let wheel = match scroll.unit {
        MouseScrollUnit::Line => scroll.delta.y * 0.25,
        MouseScrollUnit::Pixel => scroll.delta.y * 0.01,
    };
    let zoom = wheel
        + if keys.pressed(KeyCode::Equal) {
            dt
        } else {
            0.0
        }
        - if keys.pressed(KeyCode::Minus) {
            dt
        } else {
            0.0
        };
    o.dist = (o.dist * (1.0 - zoom)).clamp(0.4, 40.0);

    if let Ok(mut t) = cam.single_mut() {
        if opts.eye {
            // **First person: the camera sits where the eye is, not where a
            // viewer stands.** The body is drawn around it, which is the
            // whole question — what a player sees of themselves. Yaw and
            // pitch steer the look rather than an orbit, and `dist` is unused.
            t.translation = Vec3::new(0.0, EYE_H_M, 0.0);
            t.rotation =
                Quat::from_euler(EulerRot::YXZ, o.yaw + std::f32::consts::PI, o.pitch, 0.0);
            return;
        }
        let target = Vec3::new(0.0, o.height, 0.0);
        let dir = Vec3::new(
            o.yaw.sin() * o.pitch.cos(),
            o.pitch.sin(),
            o.yaw.cos() * o.pitch.cos(),
        );
        t.translation = target + dir * o.dist;
        t.look_at(target, Vec3::Y);
    }
}

fn keys(
    keys: Res<ButtonInput<KeyCode>>,
    mut bench: ResMut<Bench>,
    mut roots: Query<(&mut SubjectRoot, &mut Transform)>,
    mut shots: ResMut<Shots>,
) {
    if keys.just_pressed(KeyCode::BracketRight) && !bench.clips.is_empty() {
        bench.current = (bench.current + 1) % bench.clips.len();
        bench.bind_pose = false;
    }
    if keys.just_pressed(KeyCode::BracketLeft) && !bench.clips.is_empty() {
        bench.current = (bench.current + bench.clips.len() - 1) % bench.clips.len();
        bench.bind_pose = false;
    }
    if keys.just_pressed(KeyCode::Space) {
        bench.paused = !bench.paused;
    }
    if keys.just_pressed(KeyCode::KeyT) {
        bench.bind_pose = !bench.bind_pose;
    }
    if keys.just_pressed(KeyCode::KeyC) {
        bench.show_capsule = !bench.show_capsule;
    }
    if keys.just_pressed(KeyCode::KeyG) {
        bench.show_grid = !bench.show_grid;
    }
    if keys.just_pressed(KeyCode::KeyP) {
        shots.want += 1;
    }
    if keys.just_pressed(KeyCode::KeyR) {
        for (mut sub, mut t) in &mut roots {
            sub.rot_x = if sub.rot_x.abs() < 0.01 {
                FRAC_PI_2
            } else {
                0.0
            };
            t.rotation = Quat::from_rotation_x(sub.rot_x);
        }
        // Re-measure: the numbers printed before the rotation describe a
        // different pose, and a stale measurement is worse than none.
        for s in &mut bench.subjects {
            s.measured = false;
        }
    }
}

/// The sim's volumes, drawn where the mesh is. Gizmos rather than meshes so
/// nothing here can be mistaken for the asset.
fn draw(bench: Res<Bench>, roots: Query<(&SubjectRoot, &GlobalTransform)>, mut gizmos: Gizmos) {
    if bench.show_grid {
        let grey = Color::srgba(1.0, 1.0, 1.0, 0.10);
        for i in -10..=10 {
            let v = i as f32 * 0.5;
            gizmos.line(Vec3::new(v, 0.0, -5.0), Vec3::new(v, 0.0, 5.0), grey);
            gizmos.line(Vec3::new(-5.0, 0.0, v), Vec3::new(5.0, 0.0, v), grey);
        }
        // +Z is where a body with yaw 0 faces (`bodies::wire_yaw_to_radians`).
        gizmos.line(
            Vec3::ZERO,
            Vec3::new(0.0, 0.0, 1.0),
            Color::srgb(0.3, 0.6, 1.0),
        );
        gizmos.line(
            Vec3::ZERO,
            Vec3::new(1.0, 0.0, 0.0),
            Color::srgb(1.0, 0.4, 0.4),
        );
    }
    if !bench.show_capsule {
        return;
    }
    for (sub, gt) in &roots {
        let c = gt.translation();
        let yellow = Color::srgb(0.95, 0.85, 0.25);
        let cyan = Color::srgb(0.3, 0.9, 0.9);
        // The capsule the server shoots at: two rings at the cap centres, four
        // uprights, and a ring on the ground for the footprint.
        for y in [0.0, BODY_R_M, BODY_H_M - BODY_R_M, BODY_H_M] {
            ring(&mut gizmos, c + Vec3::Y * y, BODY_R_M, yellow);
        }
        for i in 0..8 {
            let a = i as f32 / 8.0 * TAU;
            let off = Vec3::new(BODY_R_M * a.cos(), 0.0, BODY_R_M * a.sin());
            gizmos.line(
                c + off + Vec3::Y * BODY_R_M,
                c + off + Vec3::Y * (BODY_H_M - BODY_R_M),
                yellow,
            );
        }
        // Eye height — where the first-person camera sits.
        ring(&mut gizmos, c + Vec3::Y * EYE_H_M, BODY_R_M * 0.45, cyan);
        // A half-metre rule up the front, so height is readable off the frame.
        for i in 0..=(BODY_H_M / 0.5) as i32 {
            let y = i as f32 * 0.5;
            gizmos.line(
                c + Vec3::new(0.0, y, BODY_R_M),
                c + Vec3::new(0.0, y, BODY_R_M + 0.12),
                Color::srgba(1.0, 1.0, 1.0, 0.5),
            );
        }
        let _ = sub;
    }
}

fn ring(gizmos: &mut Gizmos, c: Vec3, r: f32, color: Color) {
    const SEG: usize = 32;
    let pts = (0..=SEG).map(|i| {
        let a = i as f32 / SEG as f32 * TAU;
        c + Vec3::new(r * a.cos(), 0.0, r * a.sin())
    });
    gizmos.linestrip(pts, color);
}

fn hud(bench: Res<Bench>, mut text: Query<&mut Text, With<HudText>>) {
    let Ok(mut t) = text.single_mut() else { return };
    let clip = if bench.bind_pose {
        "bind pose".to_string()
    } else {
        match bench.clips.get(bench.current) {
            Some(c) => format!(
                "{c}   [{}/{}]{}",
                bench.current + 1,
                bench.clips.len(),
                if bench.paused { "  paused" } else { "" }
            ),
            None => "no clips".to_string(),
        }
    };
    let subjects: Vec<String> = bench
        .subjects
        .iter()
        .map(|s| s.name.rsplit('/').next().unwrap_or(&s.name).to_string())
        .collect();
    **t = format!(
        "{}\nclip  {clip}\n[ ] clip | space pause | t bind pose | r rot-x | c capsule | g grid | p shot\nyellow = the {BODY_H_M} m capsule the server shoots at | cyan = eye at {EYE_H_M} m",
        subjects.join("   |   ")
    );
}

/// Screenshots: a key a person presses, and the `--shot` sequence.
#[derive(Resource, Default)]
struct Shots {
    /// Presses not yet taken.
    want: u32,
    taken: u32,
    frame: u64,
    /// The frame every subject was measured on — the observable this bench
    /// settles against. `None` until then.
    settled: Option<u64>,
    /// Which bearing of the `--shot` sequence is next.
    step: usize,
}

/// The `p` key. Separate from the `--shot` sequence and running always: one is
/// a person pressing a key at a moment they chose, the other is a fixed walk
/// that exits when it is done (`render/shot.rs` draws the same line).
fn snap(mut commands: Commands, mut shots: ResMut<Shots>) {
    while shots.want > 0 {
        shots.want -= 1;
        let n = shots.taken;
        let path = std::env::current_dir()
            .unwrap_or_default()
            .join(format!("modelview-{n:03}.png"));
        println!("modelview: shot → {}", path.display());
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path));
        shots.taken += 1;
    }
}

/// The bearings `--shot` walks, radians of yaw. Front, three-quarter, side,
/// back — the four a silhouette can hide a defect behind.
const SHOT_YAWS: [f32; 4] = [0.0, 0.9, FRAC_PI_2, std::f32::consts::PI];
/// The pitches `--eye --shot` walks: level, then down, because a first-person
/// body only enters frame when the player looks down at it.
const SHOT_PITCHES: [f32; 4] = [-0.15, -0.55, -0.95, -1.35];
/// Frames to settle before the first shot, and between shots. A frame count
/// rather than a duration — `render/shot.rs`'s reason: the readback is
/// asynchronous in frames, and elapsed milliseconds measure the box.
const SHOT_SETTLE: u64 = 90;
const SHOT_GAP: u64 = 24;

fn shoot(
    mut commands: Commands,
    mut shots: ResMut<Shots>,
    mut o: ResMut<Orbit>,
    opts: Res<Opts>,
    mut bench: ResMut<Bench>,
    mut exit: MessageWriter<AppExit>,
) {
    shots.frame += 1;
    let Some(dir) = opts.shot.clone() else { return };
    if shots.frame == 1 {
        if let Err(e) = std::fs::create_dir_all(&dir) {
            eprintln!("modelview: {}: {e}", dir.display());
            exit.write(AppExit::error());
            return;
        }
    }
    // Nothing is shot until every subject has been measured — the same
    // "settle on observable state, never on a clock" rule the capture harness
    // holds, applied to the one observable this bench has. The frame count
    // after that is the pipeline warm-up, which genuinely is frames.
    let measured = !bench.subjects.is_empty() && bench.subjects.iter().all(|s| s.measured);
    if shots.settled.is_none() {
        if !measured {
            return;
        }
        shots.settled = Some(shots.frame);
    }
    let since = shots.frame - shots.settled.unwrap_or(shots.frame);
    if since < SHOT_SETTLE || !(since - SHOT_SETTLE).is_multiple_of(SHOT_GAP) {
        return;
    }
    // Two walks, and which one runs is what `--per-clip` chooses. Bearings
    // answer "is the mesh right"; clips answer "is the SET right", which is
    // the question an animation delivery actually raises — a name list tells
    // you seven clips arrived and nothing about whether one of them is the
    // idle you asked for.
    let steps = if opts.per_clip {
        bench.clips.len()
    } else {
        SHOT_YAWS.len()
    };
    if shots.step >= steps {
        println!("modelview: {} shot(s) → {}", shots.taken, dir.display());
        exit.write(AppExit::Success);
        return;
    }
    // Aim, then shoot on the NEXT visit — a camera moved this frame has not
    // been drawn from yet, and a clip switched this frame is still cross-
    // fading out of the last one.
    let yaw = if opts.per_clip {
        0.7
    } else {
        SHOT_YAWS[shots.step]
    };
    let name = if opts.per_clip {
        bench.clips.get(shots.step).cloned().unwrap_or_default()
    } else {
        format!("view{}", shots.step)
    };
    if (o.yaw - yaw).abs() > 1e-3
        || opts.eye && shots.step > 0 && (o.pitch - SHOT_PITCHES[shots.step]).abs() > 1e-3
        || (opts.per_clip && bench.current != shots.step)
    {
        o.yaw = yaw;
        o.height = BODY_H_M * 0.52;
        o.dist = 2.7;
        bench.current = shots.step;
        return;
    }
    // A filename is a person's index into a directory, so it carries the clip
    // name — sanitised, because a clip may be called anything at all.
    let stem: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let path = dir.join(format!("{:02}-{stem}.png", shots.step));
    println!("modelview: shot → {}", path.display());
    commands
        .spawn(Screenshot::primary_window())
        .observe(save_to_disk(path));
    shots.taken += 1;
    shots.step += 1;
}
