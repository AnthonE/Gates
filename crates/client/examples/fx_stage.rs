//! `fx_stage` — one effect at a time on the fx stage (`fx::stage`): fired in
//! front of a dark card and a light card at once, under the game's sun,
//! exposure and tonemap, on a clock that steps 1/60 s and holds for each
//! frame it saves. What it fires is the game's own (`render::fx::stage`:
//! a real `Contact` through `Fx::impact`), so a strip is what a blow throws.
//!
//! ```text
//! cargo run -p client --features render,native --example fx_stage -- <out> list
//! cargo run -p client --features render,native --example fx_stage -- <out> <name>… \
//!     [--frames N] [--span S] [--knobs 'Sparks.speed=4,12;…'] [--seed N] [--bloom] [--size 768x432]
//! ```
//!
//! Writes `<out>/<name>/frames/NNNN.png` (one per step, step 0 the plate
//! before the effect fires) and `<out>/<name>/stage.json`, which
//! `ci/fxstrip.mjs frames` turns into the strip, its numbers and its diff.
//! `ci/fx.sh` runs both headless (Xvfb + lavapipe).

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use bevy::camera::Exposure;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::light::light_consts::lux;
use bevy::post_process::bloom::Bloom;
use bevy::prelude::*;
use bevy::render::view::screenshot::{save_to_disk, Screenshot, ScreenshotCaptured};
use bevy::render::view::Hdr;
use bevy::time::TimeUpdateStrategy;
use bevy::window::WindowResolution;

use ::fx::stage::{Framing, DT, WARM};
use client::render::fx::{self, stage, table, Fx};
use client::render::rig::EyeCam;

/// Frames a run's first effect waits for its pipelines; later ones reuse them.
const FIRST_WARM: u32 = WARM * 4;
/// Frames to wait for the last screenshots to land before giving up.
const TAIL_MAX: u32 = 600;
/// The cards' sRGB greys, tuned under Gates' exposure and TonyMcMapface until
/// the plate reads the stage's ~25 and ~185 (`fx::stage::DARK_SRGB`,
/// `LIGHT_SRGB`; `fxstrip.mjs` says `plate … ok`).
const CARDS: [f32; 2] = [::fx::stage::DARK_SRGB, 0.86];

struct Run {
    staged: stage::Staged,
    framing: Framing,
    steps: Vec<u32>,
    /// `(step, live glow, live soft)` per frame shot.
    shot: Vec<(u32, usize, usize)>,
}

#[derive(Resource)]
struct Plan {
    out: PathBuf,
    runs: Vec<Run>,
    run: usize,
    phase: Phase,
    /// The step the stage stands at, after this frame.
    step: u32,
    /// Whether this frame's time was a step (set the frame before).
    stepping: bool,
    seed: u32,
    knobs: String,
    bloom: bool,
    size: (u32, u32),
    asked: usize,
    saved: Arc<AtomicUsize>,
    tail: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase {
    Begin,
    Warm(u32),
    /// Shooting the run's `k`th step; `held` once the step is on screen.
    Shoot {
        k: usize,
        held: bool,
    },
    Done,
}

#[derive(Component)]
struct Card(usize);

/// The camera and its lens, which `drive` frames for each effect.
type EyeLens<'w, 's> =
    Query<'w, 's, (&'static mut Transform, &'static mut Projection), (With<EyeCam>, Without<Card>)>;

fn usage() -> AppExit {
    eprintln!(
        "fx_stage <out> list | <name>… [--frames N] [--span S] [--knobs K] [--seed N] [--bloom] [--size WxH]"
    );
    AppExit::from_code(2)
}

fn main() -> AppExit {
    let mut args = std::env::args().skip(1);
    let Some(out) = args.next() else {
        return usage();
    };
    let (mut names, mut frames, mut span, mut knobs, mut seed, mut bloom, mut size) = (
        Vec::new(),
        11usize,
        None::<f32>,
        String::new(),
        0u32,
        false,
        (768u32, 432u32),
    );
    while let Some(a) = args.next() {
        let mut val = |what: &str| args.next().unwrap_or_else(|| panic!("{a} wants {what}"));
        match a.as_str() {
            "--frames" => frames = val("a count").parse().expect("--frames N"),
            "--span" => span = Some(val("seconds").parse().expect("--span S")),
            "--knobs" => knobs = val("knobs"),
            "--seed" => seed = val("a seed").parse().expect("--seed N"),
            "--bloom" => bloom = true,
            "--size" => {
                let v = val("WxH");
                let (w, h) = v.split_once('x').expect("--size WxH");
                size = (w.parse().expect("width"), h.parse().expect("height"));
            }
            "list" => {
                for s in stage::list() {
                    println!(
                        "{:<14} {:>4.2} m {:>4.1} s  {}",
                        s.name, s.radius, s.span_s, s.what
                    );
                }
                return AppExit::Success;
            }
            _ if a.starts_with("--") => return usage(),
            _ => names.push(a),
        }
    }
    if names.is_empty() {
        return usage();
    }
    if !knobs.is_empty() {
        if let Err(e) = table::set_knobs(&knobs) {
            eprintln!("fx_stage: {e}");
            return AppExit::from_code(2);
        }
    }
    let mut runs = Vec::new();
    for n in &names {
        let Some(staged) = stage::find(n) else {
            eprintln!("fx_stage: no effect `{n}` (fx_stage <out> list)");
            return AppExit::from_code(2);
        };
        let steps = ::fx::stage::steps(span.unwrap_or(staged.span_s), frames);
        runs.push(Run {
            framing: Framing::new(staged.radius),
            staged,
            steps,
            shot: Vec::new(),
        });
    }
    let out = PathBuf::from(out);
    let assets = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: "gates — fx_stage".into(),
                    resolution: WindowResolution::new(size.0, size.1),
                    ..default()
                }),
                ..default()
            })
            .set(AssetPlugin {
                file_path: assets.to_string_lossy().into_owned(),
                ..default()
            }),
    );
    app.insert_resource(ClearColor(Color::BLACK));
    app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::ZERO));
    app.insert_resource(Plan {
        out,
        runs,
        run: 0,
        phase: Phase::Begin,
        step: 0,
        stepping: false,
        seed,
        knobs,
        bloom,
        size,
        asked: 0,
        saved: Arc::new(AtomicUsize::new(0)),
        tail: 0,
    });
    app.init_resource::<Fx>();
    app.add_systems(Startup, (fx::setup, set_stage));
    app.add_systems(Update, drive);
    app.add_systems(
        PostUpdate,
        (fx::draw, fx::flash).after(bevy::transform::TransformSystems::Propagate),
    );
    app.run()
}

fn set_stage(
    plan: Res<Plan>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    use client::render::{fill, rig};
    let fill_map = images.add(fill::cubemap(fill::FILL_FACE));
    let mut eye = commands.spawn((
        EyeCam,
        Camera3d::default(),
        Projection::Perspective(PerspectiveProjection::default()),
        (
            Exposure {
                ev100: rig::DAY_EV100,
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
        Transform::default(),
    ));
    if plan.bloom {
        eye.insert((Hdr, Bloom::NATURAL));
    }
    let sun = rig::to_sun_at(rig::RIG_SUN_ELEVATION, rig::RIG_SUN_AZIMUTH);
    commands.spawn((
        DirectionalLight {
            illuminance: lux::DIRECT_SUNLIGHT * 0.8,
            shadows_enabled: false,
            ..default()
        },
        Transform::default().looking_to(-sun, Vec3::Y),
    ));
    let quad = meshes.add(Rectangle::new(1.0, 1.0));
    for (i, g) in CARDS.into_iter().enumerate() {
        let mat = materials.add(StandardMaterial {
            base_color: Color::srgb(g, g, g),
            unlit: true,
            ..default()
        });
        commands.spawn((
            Card(i),
            Mesh3d(quad.clone()),
            MeshMaterial3d(mat),
            Transform::default(),
        ));
    }
}

#[allow(clippy::too_many_arguments)]
fn drive(
    mut commands: Commands,
    mut plan: ResMut<Plan>,
    mut fx: ResMut<Fx>,
    mut strategy: ResMut<TimeUpdateStrategy>,
    mut eye: EyeLens,
    mut cards: Query<(&Card, &mut Transform), Without<EyeCam>>,
    mut exit: MessageWriter<AppExit>,
) {
    let p = &mut *plan;
    // This frame's time was chosen last frame: a step or a hold.
    if p.stepping {
        p.step += 1;
        if p.step == 1 {
            let run = &p.runs[p.run];
            let f = run.framing;
            for at in f.anchors {
                stage::fire(
                    &mut fx,
                    &run.staged,
                    Vec3::from(at),
                    Vec3::from(f.dir),
                    Vec3::from(f.eye),
                );
            }
        }
    }
    let mut step_next = false;
    match p.phase {
        Phase::Begin => {
            let f = p.runs[p.run].framing;
            if let Ok((mut tf, mut proj)) = eye.single_mut() {
                *tf = Transform::from_translation(f.eye.into()).looking_at(f.look.into(), Vec3::Y);
                *proj = Projection::Perspective(PerspectiveProjection {
                    fov: f.fov_y,
                    near: 0.02,
                    far: 200.0,
                    ..default()
                });
            }
            for (Card(i), mut tf) in &mut cards {
                let c = f.cards[*i];
                // Just behind the plane the effects fire on, so a spark that
                // bounces off it is never hidden inside the card.
                *tf = Transform::from_translation(Vec3::from(c.centre) - Vec3::Z * 0.002)
                    .with_scale(Vec3::new(c.half[0] * 2.0, c.half[1] * 2.0, 1.0));
            }
            fx.clear();
            fx.glow.seed(p.seed);
            fx.soft.seed(p.seed.wrapping_mul(0x9E37_79B9));
            p.step = 0;
            p.phase = Phase::Warm(if p.run == 0 { FIRST_WARM } else { WARM });
        }
        Phase::Warm(n) => {
            p.phase = if n <= 1 {
                Phase::Shoot { k: 0, held: false }
            } else {
                Phase::Warm(n - 1)
            };
        }
        Phase::Shoot { k, held } => {
            let target = p.runs[p.run].steps[k];
            if p.step < target {
                step_next = true;
            } else if !held {
                // On screen this frame; hold one more so nothing moves under
                // the shot.
                p.phase = Phase::Shoot { k, held: true };
            } else {
                let name = p.runs[p.run].staged.name.clone();
                let path = p
                    .out
                    .join(&name)
                    .join("frames")
                    .join(format!("{target:04}.png"));
                if let Some(dir) = path.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                let saved = p.saved.clone();
                commands
                    .spawn(Screenshot::primary_window())
                    .observe(save_to_disk(path))
                    .observe(move |_: On<ScreenshotCaptured>| {
                        saved.fetch_add(1, Ordering::SeqCst);
                    });
                p.asked += 1;
                let live = (fx.glow.live(), fx.soft.live());
                p.runs[p.run].shot.push((target, live.0, live.1));
                if k + 1 < p.runs[p.run].steps.len() {
                    p.phase = Phase::Shoot {
                        k: k + 1,
                        held: false,
                    };
                    step_next = p.step < p.runs[p.run].steps[k + 1];
                } else {
                    write_run(p, p.run);
                    p.run += 1;
                    p.phase = if p.run < p.runs.len() {
                        Phase::Begin
                    } else {
                        Phase::Done
                    };
                }
            }
        }
        Phase::Done => {
            p.tail += 1;
            let saved = p.saved.load(Ordering::SeqCst);
            if saved >= p.asked || p.tail > TAIL_MAX {
                if saved < p.asked {
                    eprintln!("fx_stage: only {saved} of {} frames saved", p.asked);
                    exit.write(AppExit::error());
                } else {
                    println!("fx_stage: {saved} frames → {}", p.out.display());
                    exit.write(AppExit::Success);
                }
            }
        }
    }
    p.stepping = step_next;
    *strategy = TimeUpdateStrategy::ManualDuration(if step_next {
        Duration::from_secs_f32(DT)
    } else {
        Duration::ZERO
    });
}

/// `stage.json`: what `fxstrip.mjs frames` reads to make the strip.
fn write_run(p: &Plan, i: usize) {
    let run = &p.runs[i];
    let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    let frames: Vec<String> = run
        .shot
        .iter()
        .map(|(step, glow, soft)| {
            format!(
                "{{\"step\":{step},\"t\":{:.4},\"file\":\"frames/{step:04}.png\",\"live\":[{glow},{soft}]}}",
                step.saturating_sub(1) as f32 * DT
            )
        })
        .collect();
    let recipes: Vec<String> = table::Layer::ALL
        .iter()
        .map(|l| {
            format!(
                "\"{}\":\"{}\"",
                l.name(),
                esc(&table::recipe(*l).to_string())
            )
        })
        .collect();
    let json = format!(
        "{{\"game\":\"Gates\",\"name\":\"{}\",\"what\":\"{}\",\"hz\":{},\"size\":[{},{}],\"seed\":{},\
         \"knobs\":\"{}\",\"bloom\":{},\"radius\":{},\"span\":{},\"steps\":{:?},\"cards\":[{},{}],\
         \"frames\":[{}],\"recipes\":{{{}}}}}\n",
        esc(&run.staged.name),
        esc(&run.staged.what),
        ::fx::stage::HZ,
        p.size.0,
        p.size.1,
        p.seed,
        esc(&p.knobs),
        p.bloom,
        run.staged.radius,
        run.staged.span_s,
        run.steps,
        CARDS[0],
        CARDS[1],
        frames.join(","),
        recipes.join(","),
    );
    let path = p.out.join(&run.staged.name).join("stage.json");
    if let Err(e) = std::fs::write(&path, json) {
        eprintln!("fx_stage: {}: {e}", path.display());
    }
}
