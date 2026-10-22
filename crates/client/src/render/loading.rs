//! The loading screen: the world is built here, and this is what the player
//! looks at while it happens.
//!
//! **It is a state, not a splash, and the progress is real.** Nothing here
//! counts frames or interpolates a bar toward 100% — the three ring builders
//! each report how many of their cells are resident (`Ring`, `PropRing`,
//! `ClutterRing`), the bar is their mean, and the screen ends the instant all
//! three say full. That is the same observable-state settle the capture
//! harness uses and for the same reason: a bar driven by a clock is a bar that
//! lies on a slow box, and this repo's rule is that a gate — or a player —
//! waits on state, never on milliseconds.
//!
//! **Where it sits in the flow.** `Connecting` ends the moment the welcome
//! lands, which is when the seed exists and therefore when `WorldId` can be
//! built; but a welcome is not a world. Everything between the welcome and a
//! frame worth showing — a 5x5 ring of ground chunks, the same ring of
//! scatter, a ring of clutter tiles and the far mesh, one build of each per
//! frame by budget — used to happen with the
//! player already nominally in the world, staring at an empty near ring
//! filling in around them. This is that interval, named.
//!
//! **It covers two waits, not one, and the first one used to be invisible.**
//! The welcome names the island; the first snapshot names where on it the
//! player stands. Between those the client knows a seed and nothing else,
//! and the rings — which read `Eye::pos` — used to spend that interval
//! building the world **origin**, because an unplaced predictor reports
//! `Body::default()` and that is a real location rather than a sentinel.
//! Every connect threw those chunks away; a first snapshot slower than the
//! whole ring build would have filled the bar there.
//! `crate::ui::load::Progress::placed` is the gate for it,
//! `render::world_placed` is where the streamers stand down, and this screen
//! is what the player looks at meanwhile.
//!
//! **The world renders behind it, deliberately.** The overlay is opaque and
//! the rig is up, so the 3D pass runs for every loading frame with nothing
//! visible to the player — which is exactly when a pipeline should be paying
//! its first-use specialization cost. The screen does not end until those
//! compiles are done (`render::pipelines`).
//!
//! **The overlay has its own camera.** It used to draw on the rig's HDR
//! camera, whose UI, tonemapping and bloom pipelines are compiled on first
//! use. Natively a draw whose pipeline is not ready is skipped, so for the
//! first few seconds the player saw the rig's clear colour and no overlay: a
//! flat grey screen right after signing in. A plain `Camera2d` above the rig
//! uses the same non-HDR UI pipeline every menu screen already compiled.

use bevy::camera::{CameraOutputMode, ClearColorConfig};
use bevy::prelude::*;
use bevy::render::render_resource::BlendState;

use super::clutter::{ClutterRing, RING_TILES};
use super::pipelines::PipelineBacklog;
use super::props::PropRing;
use super::screen::{Connecting, Direct, Menu, Screen};
use super::terrain_mesh::{Ring, RING_CHUNKS};
use super::{ui, Eye, WorldId};
use crate::ui::load::Progress;

/// Everything this screen owns, so leaving despawns exactly it.
#[derive(Component)]
pub struct LoadingRoot;

/// The overlay's own camera. See the module header for why it has one.
#[derive(Component)]
pub struct LoadingCam;

/// The filled part of the bar. Its width is the only thing that animates
/// once the server has placed us.
#[derive(Component)]
pub struct Bar;

/// The sweep that runs along the empty bar while the server has not placed
/// us yet, so 0% does not read as stuck.
#[derive(Component)]
pub struct Sweep;

/// The stage line above the bar.
#[derive(Component)]
pub struct Stage;

/// The three counters, redrawn in place.
#[derive(Component)]
pub struct Counts;

/// Read this frame's [`Progress`] off the rings.
///
/// The arithmetic on it is [`crate::ui::load`]'s — pure, and gated in the
/// code tier. This is the half that has to touch the render module, and it
/// is the whole of that half: the counters and the shipped totals.
///
/// `placed` is passed in rather than read from `Eye` here so the function
/// stays a plain projection of the rings, and so the caller cannot
/// accidentally answer the two questions from two different frames.
pub fn read(
    placed: bool,
    ring: &Ring,
    props: &PropRing,
    clutter: &ClutterRing,
    pipelines: u32,
) -> Progress {
    Progress {
        placed,
        ground: (ring.len(), RING_CHUNKS),
        scatter: (props.len(), RING_CHUNKS),
        clutter: (clutter.len(), RING_TILES),
        far: ring.far_ready(),
        pipelines,
    }
}

/// Bar geometry, pixels. Not a knob in the registry sense — it is the width
/// of a rectangle, chosen to match the shard rows on the screen before it.
const BAR_W: f32 = 560.0;
const BAR_H: f32 = 10.0;

/// The sweep's width as a share of the bar, and seconds per pass. The same
/// motion as the browser page's handoff bar, so the page and this screen
/// read as one wait.
const SWEEP_W: f32 = 22.0;
const SWEEP_S: f32 = 1.4;

/// How long the overlay takes to fade into the world.
const FADE_S: f32 = 0.35;

/// Frames the world must read finished before the screen ends.
const DONE_FRAMES: u32 = 3;

/// Frames to wait with nothing but pipelines left and the count not moving
/// before entering anyway. A shader that never loads is a bug, but it must
/// not keep a player on this screen.
const PIPELINE_STALL_FRAMES: u32 = 300;

/// Short lines under the bar. One is picked by the seed, so a player sees a
/// different one on a different island. Bracketed keys are rewritten if the
/// player rebound them (`binds::Binds::relabel`).
const TIPS: [&str; 6] = [
    "Hold [G] to open the island map. Right click marks a spot.",
    "[TAB] opens your inventory, [Q] opens crafting.",
    "Crouch ([CTRL]) to move quietly. Animals listen for footsteps.",
    "[O] shows the works the whole server is switching back on.",
    "Press [T] to chat. [N] invites the player you are looking at to a team.",
    "A hearth claims the ground around it. Keep it fed.",
];

pub fn setup(
    mut commands: Commands,
    connecting: NonSend<Connecting>,
    menu: Res<Menu>,
    direct: Option<Res<Direct>>,
    world: Option<Res<WorldId>>,
    binds: Res<super::binds::Binds>,
) {
    // The server's listed name when the menu has one for this address, else
    // the address itself. A browser start has no menu rows and no connect
    // screen, so it falls back to the address it was started with.
    let addr = if connecting.addr.is_empty() {
        direct.map(|d| d.0.clone()).unwrap_or_default()
    } else {
        connecting.addr.clone()
    };
    let name = menu
        .rows
        .iter()
        .find(|r| !r.direct && r.addr == addr)
        .map(|r| r.name.clone())
        .unwrap_or(addr);
    let title = if name.is_empty() || crate::ui::streamer::on() {
        "ENTERING THE ISLAND".to_string()
    } else {
        format!("ENTERING {}", name.to_uppercase())
    };
    let tip = TIPS[world.map(|w| w.seed as usize % TIPS.len()).unwrap_or(0)];

    let cam = commands
        .spawn((
            LoadingCam,
            Camera2d,
            // Off, never Bevy's default 4×. Cameras drawing to one target
            // share its non-HDR main texture, and a scaled world's
            // presentation camera (`render_scale`) is that and `Msaa::Off`:
            // wgpu refuses the pass that pairs this camera's 4-sample depth
            // with the shared 1-sample colour, and the page panicked on its
            // first loading frame. A UI layer gains nothing from it anyway.
            Msaa::Off,
            Camera {
                // Above the rig, and blended over it rather than replacing
                // it, so the fade at the end shows the world underneath.
                order: 1,
                clear_color: ClearColorConfig::Custom(Color::NONE),
                output_mode: CameraOutputMode::Write {
                    blend_state: Some(BlendState::ALPHA_BLENDING),
                    clear_color: ClearColorConfig::None,
                },
                ..default()
            },
        ))
        .id();

    commands.spawn((
        LoadingRoot,
        UiTargetCamera(cam),
        ui::screen(ui::BG),
        children![
            ui::wordmark(),
            (
                ui::strong(title, 18.0, ui::TEXT),
                Node {
                    margin: UiRect::top(Val::Px(18.0)),
                    ..default()
                },
            ),
            (Stage, ui::label(Progress::waiting().stage(), 15.0, ui::DIM)),
            // The track, the sweep and the fill inside it.
            (
                Node {
                    width: Val::Px(BAR_W),
                    height: Val::Px(BAR_H),
                    margin: UiRect::vertical(Val::Px(10.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    overflow: Overflow::clip(),
                    ..default()
                },
                BackgroundColor(ui::ROW_IDLE),
                BorderColor::all(ui::RULE),
                children![
                    (
                        Bar,
                        Node {
                            width: Val::Percent(0.0),
                            height: Val::Percent(100.0),
                            ..default()
                        },
                        BackgroundColor(ui::ACCENT),
                    ),
                    (
                        Sweep,
                        Node {
                            position_type: PositionType::Absolute,
                            width: Val::Percent(SWEEP_W),
                            height: Val::Percent(100.0),
                            left: Val::Percent(-SWEEP_W),
                            ..default()
                        },
                        BackgroundColor(ui::ACCENT),
                    ),
                ],
            ),
            (Counts, ui::label("", 12.0, ui::FAINT)),
            (
                ui::label(binds.relabel(tip), 14.0, ui::DIM),
                Node {
                    margin: UiRect::top(Val::Px(22.0)),
                    ..default()
                },
            ),
            ui::label("Esc cancels", 12.0, ui::FAINT),
        ],
    ));
}

/// Drive the bar, and end the screen when the world is actually there.
#[allow(clippy::too_many_arguments)]
pub fn update(
    eye: Res<Eye>,
    ring: Res<Ring>,
    props: Res<PropRing>,
    clutter: Res<ClutterRing>,
    backlog: Res<PipelineBacklog>,
    time: Res<Time>,
    mut bar: Query<&mut Node, (With<Bar>, Without<Sweep>)>,
    mut sweep: Query<&mut Node, (With<Sweep>, Without<Bar>)>,
    mut stage: Query<&mut Text, (With<Stage>, Without<Counts>)>,
    mut counts: Query<&mut Text, (With<Counts>, Without<Stage>)>,
    keyboard: Res<ButtonInput<KeyCode>>,
    connecting: NonSend<Connecting>,
    mut menu: ResMut<Menu>,
    mut next: ResMut<NextState<Screen>>,
    town: Option<Res<super::town::TownModel>>,
    (mut stall, mut held): (Local<(u32, u32)>, Local<u32>),
) {
    let p = read(eye.placed, &ring, &props, &clutter, backlog.pending());

    if let Ok(mut node) = bar.single_mut() {
        node.width = Val::Percent(p.fraction() * 100.0);
    }
    if let Ok(mut node) = sweep.single_mut() {
        if p.placed {
            node.display = Display::None;
        } else {
            let t = (time.elapsed_secs() / SWEEP_S).fract();
            node.left = Val::Percent(-SWEEP_W + t * (100.0 + SWEEP_W));
        }
    }
    if let Ok(mut text) = stage.single_mut() {
        if text.0 != p.stage() {
            text.0 = p.stage().to_string();
        }
    }
    if let Ok(mut text) = counts.single_mut() {
        let line = if p.placed { p.line() } else { String::new() };
        if text.0 != line {
            text.0 = line;
        }
    }

    // The way out. A world that never finishes building is a bug rather than
    // a wait, but the player still has to be able to leave one — the same
    // rule the connect screen obeys.
    if keyboard.just_pressed(KeyCode::Escape) {
        menu.status = format!(
            "{}: left while the world was loading",
            crate::ui::streamer::server(&connecting.addr)
        );
        next.set(Screen::Menu);
        return;
    }

    // The stall guard: everything but the pipelines is ready and their
    // count has not moved for a while. Enter anyway, and say so in the log.
    let (last, frames) = &mut *stall;
    if p.built() && p.pipelines > 0 && p.pipelines == *last {
        *frames += 1;
    } else {
        *frames = 0;
    }
    *last = p.pipelines;
    let stalled = *frames >= PIPELINE_STALL_FRAMES;
    if stalled {
        warn!(
            "loading: {} pipelines still pending after {PIPELINE_STALL_FRAMES} frames; entering anyway",
            p.pipelines
        );
    }

    // THE GATE's dressed model too (7 MB): drawn at any distance, so a swap
    // from its boxes would happen in plain sight.
    //
    // Held for a few frames, because the pipeline count is the render
    // world's and arrives a frame late: the last ring cell can queue a new
    // pipeline on the same frame the rings read full.
    if (p.done() || stalled) && town.is_none_or(|t| t.ready()) {
        *held += 1;
    } else {
        *held = 0;
    }
    if *held >= DONE_FRAMES {
        next.set(Screen::InWorld);
    }
}

/// An overlay on its way out: alpha goes from 1 to 0 over [`FADE_S`], then
/// the overlay and its camera are despawned.
#[derive(Component)]
pub struct Fading {
    t: f32,
    /// The factor applied so far, so each frame scales by the ratio.
    k: f32,
}

/// Leaving: fade into the world rather than cut. A capture run cuts, because
/// its frames must not catch a half-faded overlay.
pub fn teardown(
    mut commands: Commands,
    roots: Query<Entity, With<LoadingRoot>>,
    cams: Query<Entity, With<LoadingCam>>,
    capture: Option<Res<super::capture::Capture>>,
) {
    for e in roots.iter() {
        if capture.is_some() {
            commands.entity(e).despawn();
        } else {
            commands.entity(e).insert(Fading { t: 0.0, k: 1.0 });
        }
    }
    if capture.is_some() {
        for e in cams.iter() {
            commands.entity(e).despawn();
        }
    }
}

/// Runs every frame on whatever is fading. Bevy UI has no group opacity, so
/// every colour under the root is scaled by the same factor.
#[allow(clippy::type_complexity)]
pub fn fade(
    mut commands: Commands,
    time: Res<Time>,
    mut roots: Query<(Entity, &mut Fading)>,
    children: Query<&Children>,
    mut colours: Query<(
        Option<&mut BackgroundColor>,
        Option<&mut BorderColor>,
        Option<&mut TextColor>,
        Option<&mut ImageNode>,
    )>,
    cams: Query<Entity, With<LoadingCam>>,
) {
    for (root, mut f) in roots.iter_mut() {
        f.t += time.delta_secs();
        let k = (1.0 - f.t / FADE_S).max(0.0);
        if k <= 0.0 {
            commands.entity(root).despawn();
            for cam in cams.iter() {
                commands.entity(cam).despawn();
            }
            continue;
        }
        let r = k / f.k;
        f.k = k;
        let scale = |c: &mut Color| {
            let a = c.alpha();
            c.set_alpha(a * r);
        };
        for e in std::iter::once(root).chain(children.iter_descendants(root)) {
            let Ok((bg, border, text, image)) = colours.get_mut(e) else {
                continue;
            };
            if let Some(mut c) = bg {
                scale(&mut c.0);
            }
            if let Some(mut c) = border {
                scale(&mut c.top);
                scale(&mut c.right);
                scale(&mut c.bottom);
                scale(&mut c.left);
            }
            if let Some(mut c) = text {
                scale(&mut c.0);
            }
            if let Some(mut c) = image {
                scale(&mut c.color);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fixture built from the SHIPPED ring sizes rather than from numbers
    /// typed here — a test carrying its own 25 would keep passing after
    /// `NEAR_RADIUS` changed, asserting about a bar nobody draws.
    ///
    /// **Placed**, because these three are about the ring arithmetic and the
    /// placement gate is `tests/ui.rs` §E's, where it runs in the code tier.
    /// A fixture that left it false would make every assertion below pass on
    /// the gate rather than on the thing it names.
    fn p(g: usize, s: usize, c: usize, far: bool) -> Progress {
        Progress {
            placed: true,
            ground: (g, RING_CHUNKS),
            scatter: (s, RING_CHUNKS),
            clutter: (c, RING_TILES),
            far,
            pipelines: 0,
        }
    }
    fn full() -> Progress {
        p(RING_CHUNKS, RING_CHUNKS, RING_TILES, true)
    }

    #[test]
    fn the_bar_is_the_mean_of_three_rings() {
        // One ring at full cannot carry the bar past a third, which is the
        // whole reason the mean is taken rather than the max.
        assert!((p(RING_CHUNKS, 0, 0, true).fraction() - 1.0 / 3.0).abs() < 1e-6);
        assert_eq!(p(0, 0, 0, false).fraction(), 0.0);
        assert_eq!(full().fraction(), 1.0);
    }

    #[test]
    fn a_full_bar_is_not_a_built_world() {
        // Every ring resident but the far mesh still missing: the bar reads
        // 100% and the screen must NOT end, because the far mesh is what the
        // player sees past 160 m. `Ring::is_full` has always carried this;
        // the bar would hide it.
        let all_but_far = p(RING_CHUNKS, RING_CHUNKS, RING_TILES, false);
        assert_eq!(all_but_far.fraction(), 1.0);
        assert!(!all_but_far.done());
        assert!(full().done());
    }

    #[test]
    fn a_ring_that_over_fills_cannot_push_the_bar_past_full() {
        assert_eq!(p(999, 999, 999, true).fraction(), 1.0);
    }

    /// The seam between this module and the pure one: `read` is a projection
    /// and must not have an opinion. It is the only place the placement
    /// answer can be dropped on the floor, and dropping it would restore
    /// exactly the bug the gate exists for — silently, with every assertion
    /// above still green.
    #[test]
    fn read_carries_the_placement_answer_through() {
        let (r, pr, c) = (Ring::default(), PropRing::default(), ClutterRing::default());
        assert!(!read(false, &r, &pr, &c, 0).placed);
        assert!(read(true, &r, &pr, &c, 0).placed);
        assert_eq!(read(false, &r, &pr, &c, 0).ground.1, RING_CHUNKS);
        assert_eq!(read(true, &r, &pr, &c, 7).pipelines, 7);
    }
}
