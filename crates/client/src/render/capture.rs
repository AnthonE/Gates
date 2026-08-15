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

/// Frames to wait for the server to place the player before giving up.
///
/// The rings cannot fill until the first snapshot carrying our own entity
/// lands (`render::world_placed`), so the settle below now has a precondition
/// it did not have when it only waited on ring counts. **An unbounded wait on
/// a precondition is a gate that hangs**, and a hung gate reports nothing at
/// all — strictly worse than a red one, which is the same reasoning as the
/// file check at the end of this file.
///
/// A frame count rather than a timeout, for the reason every other budget
/// here is one: this box shares cores and the renderer is a CPU rasterizer,
/// so elapsed milliseconds measure the box and not the client. Placement is
/// one round trip after a connect that has already completed, so anything
/// past a few dozen frames is a broken shard rather than a slow one; the
/// budget is loose enough that only "never" reaches it.
pub const PLACE_FRAMES: u32 = 300;

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
///
/// ⚠ **`1` and `3` SWAPPED LABELS on 2026-08-15 and kept their yaws**
/// (`DECISIONS.md`). Both were mislabelled: yaw `+π/2` turns the view left
/// and east is `-X`, so the camera called `east` was looking west and the one
/// called `west` was looking east. Swapping the labels rather than the yaws is
/// deliberate — the index is what a past `-visual.md` cites, so every frame
/// ever shot from vantage 1 is still the same camera, while every future one
/// is labelled truthfully. The whole cost of that bug was a wrong label;
/// keeping a known-wrong one to protect citations is the wrong trade.
///
/// The rule the harness rests on stands: **add a vantage, never silently
/// re-point one** (`gates-loop/GOAL.md`). No yaw or pitch moved here.
pub const VANTAGES: [(&str, f32, f32); 6] = [
    ("design", 0.0, -0.15),
    ("west", std::f32::consts::FRAC_PI_2, -0.15),
    ("south", std::f32::consts::PI, -0.15),
    ("east", -std::f32::consts::FRAC_PI_2, -0.15),
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
// Eight, and the eighth is `Eye` — the harness's own precondition. It settles
// on three ring counts, and a ring count is only evidence about the world once
// the server has said where that world is (`PLACE_FRAMES`); reading four
// observables rather than three is what keeps the settle honest instead of
// patient.
#[allow(clippy::too_many_arguments)]
pub fn drive(
    mut commands: Commands,
    mut cap: ResMut<Capture>,
    mut look: ResMut<Look>,
    mut exit: MessageWriter<AppExit>,
    eye: Res<super::Eye>,
    ring: Res<Ring>,
    props: Res<PropRing>,
    clutter: Res<ClutterRing>,
) {
    cap.frame += 1;
    look.frozen = true;

    // **Nothing is built until the server says where.** The welcome carries a
    // seed and no position, so the streamers stand down until the first
    // snapshot places us and the ring counts below are all zero meanwhile.
    // Waiting here is correct; waiting forever is not.
    if !eye.placed {
        if cap.frame >= PLACE_FRAMES {
            eprintln!(
                "capture: the shard never placed us — no snapshot carrying our own \
                 entity in {PLACE_FRAMES} frames. Nothing was built and nothing was shot."
            );
            exit.write(AppExit::error());
        }
        return;
    }

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
            // **Verify the files, because the writer cannot fail loudly.**
            // `save_to_disk` handles an IO error with `error!("Cannot save
            // screenshot, IO error: {e}")` and then simply returns — there is
            // no error path back to the caller. `cap.taken` counts screenshot
            // entities SPAWNED, not files landed, so without this a capture
            // run can exit 0 having written nothing at all, and a gate reading
            // the directory would find it empty and have no idea whether that
            // meant "broken renderer" or "broken disk". That is the worst bug
            // class in this repo's trap list: a pass it did not earn.
            let mut missing = Vec::new();
            for (idx, (label, _, _)) in VANTAGES.iter().enumerate() {
                let path = cap.dir.join(format!("{idx}-{label}.png"));
                // `is_file()` as well as non-empty: a directory reports a
                // non-zero length, so a size check alone would accept one.
                match std::fs::metadata(&path) {
                    Ok(m) if m.is_file() && m.len() > 0 => {}
                    _ => missing.push(path),
                }
            }
            if missing.is_empty() {
                println!(
                    "capture: {} frame(s) written to {}",
                    cap.taken,
                    cap.dir.display()
                );
                exit.write(AppExit::Success);
            } else {
                for p in &missing {
                    eprintln!("capture: MISSING or empty: {}", p.display());
                }
                eprintln!(
                    "capture: {} of {} vantages did not reach disk",
                    missing.len(),
                    VANTAGES.len()
                );
                exit.write(AppExit::error());
            }
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
