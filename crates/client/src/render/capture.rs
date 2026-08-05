//! Capture mode: `gates --capture <dir>`.
//!
//! The client's own probe harness, and the reason it exists is
//! `MIGRATION.md`'s rule, restated in `CLAUDE.md`: a render path that lands
//! without its probes ships a client with no visual gates at all.
//!
//! Three properties, each one a trap already paid for:
//!
//!   · **It settles on observable state, never on a clock.** The rings report
//!     when they are full; nothing here reads elapsed milliseconds. A gate
//!     that waits on a wall clock is not a gate on a box that shares cores,
//!     and under a CPU rasterizer it is not a gate anywhere.
//!   · **It budgets in FRAMES.** lavapipe renders at a rate that says nothing
//!     about a GPU, so every number below is a frame count.
//!   · **One live renderer, fixed seed, fixed spawn, fresh process per run.**
//!     Two live renderers was the browser tier's whole problem.
//!
//! The vantage list is `ci/vantages.mjs`'s hard-won lesson in its shortest
//! form: not one frame, from one spawn, at one bearing. Three defects shipped
//! green through the beach's blind spot because every material assertion in
//! `browser_smoke` fired from a single yaw.

use bevy::prelude::*;
use bevy::render::view::screenshot::{save_to_disk, Screenshot};
use std::path::PathBuf;

use super::clutter::ClutterRing;
use super::input::Look;
use super::props::PropRing;
use super::terrain_mesh::Ring;

/// Frames to hold after the world is built before the first shot. Pipelines
/// specialize lazily on first draw — the same class as the browser's lazy
/// WebGL program links, which read 90 fps on a benchmark and cost 700 ms of
/// worst-frame in real play. Holding here means every shot is taken on warm
/// pipelines rather than the first one paying for all of them.
pub const WARM_FRAMES: u32 = 30;
/// Frames between shots, so a vantage's first frame is never its own warmup.
pub const FRAMES_PER_SHOT: u32 = 6;
/// Frames to hold after the last shot so the async readback lands on disk.
pub const TAIL_FRAMES: u32 = 20;

/// `(label, yaw radians, pitch radians)`.
pub const VANTAGES: [(&str, f32, f32); 6] = [
    ("design", 0.0, -0.15),
    ("east", std::f32::consts::FRAC_PI_2, -0.15),
    ("south", std::f32::consts::PI, -0.15),
    ("west", -std::f32::consts::FRAC_PI_2, -0.15),
    ("near", 0.7, -0.85),
    ("sky", 2.35, 0.35),
];

#[derive(Resource)]
pub struct Capture {
    pub dir: PathBuf,
    taken: usize,
    /// Frames since the world reported itself built.
    since_built: u32,
    built: bool,
    finished_at: Option<u32>,
    frame: u32,
}

impl Capture {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            taken: 0,
            since_built: 0,
            built: false,
            finished_at: None,
            frame: 0,
        }
    }
}

/// Aim the view for the shot that is about to be taken, and take it.
pub fn drive(
    mut commands: Commands,
    mut cap: ResMut<Capture>,
    mut look: ResMut<Look>,
    mut exit: MessageWriter<AppExit>,
    ring: Res<Ring>,
    props: Res<PropRing>,
    clutter: Res<ClutterRing>,
) {
    cap.frame += 1;
    look.frozen = true;

    // Observable state: every ring the streamers own is full. Each fills one
    // entry a frame, so this is ~25 frames, and it is a COUNT of built things
    // rather than a guess about how long that takes.
    if !cap.built {
        if ring.is_full() && props.is_full() && clutter.is_full() {
            cap.built = true;
            println!(
                "capture: world built at frame {} — {} chunks, {} scatter, {} clutter tiles",
                cap.frame,
                ring.len(),
                props.len(),
                clutter.len()
            );
        } else {
            // Aim at the first vantage while the world builds, so the frames
            // that warm the pipelines are the frames that will be shot.
            let (_, yaw, pitch) = VANTAGES[0];
            look.yaw = yaw;
            look.pitch = pitch;
            return;
        }
    }

    cap.since_built += 1;
    if cap.since_built < WARM_FRAMES {
        return;
    }

    if let Some(at) = cap.finished_at {
        if cap.frame >= at + TAIL_FRAMES {
            println!(
                "capture: {} frame(s) written to {}",
                cap.taken,
                cap.dir.display()
            );
            exit.write(AppExit::Success);
        }
        return;
    }

    let phase = (cap.since_built - WARM_FRAMES) % FRAMES_PER_SHOT;
    let idx = ((cap.since_built - WARM_FRAMES) / FRAMES_PER_SHOT) as usize;
    if idx >= VANTAGES.len() {
        cap.finished_at = Some(cap.frame);
        return;
    }

    let (label, yaw, pitch) = VANTAGES[idx];
    look.yaw = yaw;
    look.pitch = pitch;

    // Aim on the phase's first frame, shoot on its last: the camera has to
    // have been at this bearing for a frame before the frame is worth reading.
    if phase == FRAMES_PER_SHOT - 1 {
        let path = cap.dir.join(format!("{idx}-{label}.png"));
        println!("capture: {}", path.display());
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path));
        cap.taken += 1;
    }
}
