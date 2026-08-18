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
use sim_core::terrain;

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

/// Cells either side of the probe searched for something to swing at.
///
/// Two, so the walk is at most ~24 m — long enough that a spawn point with
/// no node in its own cell still finds one, short enough that the budget
/// below is not a euphemism for "forever".
pub const QUARRY_CELLS: i32 = 2;
/// Frames the probe may spend walking at its quarry before giving up.
///
/// A frame count, like every other budget here, and for the same reason:
/// under a CPU rasterizer elapsed milliseconds measure the box. The walk
/// is server-side at `movement::WALK_SPEED` and the client sends one input
/// frame per rendered frame, so this is generous by design — a probe that
/// gives up early would report "no mark" for a mark that was coming.
pub const WALK_FRAMES: u32 = 240;
/// Frames of no measurable approach that count as "as close as collision
/// will let this body get". Small, because the probe's frames are huge:
/// under lavapipe this is a handful of seconds of a body pressed against
/// whatever stopped it.
pub const STALL_FRAMES: u32 = 20;
/// Frames the probe may hold the swing before giving up on ever seeing a
/// mark — a BOUND, not the plan.
///
/// **The stop is observable state: the first impact the client hears.**
/// That is not fussiness, it is the difference between a photograph of the
/// thing and a photograph of where the thing used to be. `gather::swing`
/// pays one swing per `SWING_INTERVAL_TICKS` (1.267 s) while a probe frame
/// under lavapipe is about a second, so a budget of 45 frames is ~35
/// swings — measured, and it harvested the boulder beside spawn to
/// nothing before the shutter opened. The mark lands on the FIRST swing;
/// every one after it is spent destroying the surface the mark is on.
pub const SWING_FRAMES: u32 = 45;
/// Frames between the swing pass ending and its shot, so the decal that
/// swing produced has crossed the wire and been drawn.
pub const MARK_SETTLE_FRAMES: u32 = 12;

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

/// What the probe wants the player to do this frame, in the player's own
/// terms — the same two axes and the same button mask a keyboard produces.
///
/// It exists because a capture that can only LOOK cannot photograph a verb,
/// and every mark, swing arc and built piece in this game is the result of
/// one. `render::input::gather` reads it instead of the keyboard while it
/// is `Some`, which keeps the wire path identical: the probe is a player
/// with no hands, not a second way into `set_input`.
#[derive(Clone, Copy, Default)]
pub struct Intent {
    /// The wire's own movement axes, ±127, **not** a pair of key states.
    ///
    /// The sim reads these analog — `movement::step` divides by 127 — and
    /// the probe needs that range, because a keyboard's ±1 is uncontrollable
    /// at this frame rate. Under lavapipe a frame is about a second, the
    /// server applies the last input it received on every one of its 30
    /// ticks, and a body at `WALK_SPEED` therefore covers ~3 m between two
    /// probe frames: full throttle at a quarry 2.3 m away walks straight
    /// past it, turns round, and walks past it again forever. Measured, not
    /// predicted — the first cut of this pass did exactly that and burned
    /// its whole budget oscillating.
    pub move_x: i8,
    pub move_z: i8,
    pub buttons: u8,
}

/// How far the probe has got through the verb pass that follows the
/// vantages. Ordered, and each arm ends by naming the next one.
#[derive(Clone, Copy, PartialEq)]
enum Verb {
    /// Nothing chosen yet — the scan below runs once.
    Hunt,
    /// Walking at a quarry at `(x, z)`, since frame `_`, with the closest
    /// approach so far and the frame it was last improved on.
    ///
    /// **The stall is the arrival signal, not a distance.** A body is
    /// stopped by collision at the occupant's radius plus the capsule's,
    /// which is a different number for a tree, a rock and a stone node —
    /// so any threshold picked here is right for one of them. Measured:
    /// with a threshold of 0.9 × `REACH_M` the probe converged on the
    /// shipped seed's boulder at exactly 1.8 m and sat there until its
    /// budget ran out, because 1.8 m IS that rock's standoff.
    Walk {
        x: f32,
        z: f32,
        since: u32,
        best: f32,
        improved: u32,
    },
    /// In reach and swinging, since frame `_`.
    Swing { since: u32 },
    /// Swung; waiting for the mark to arrive, then shooting it.
    Settle { since: u32 },
    /// Over, either shot or honestly skipped.
    Done,
}

#[derive(Resource)]
pub struct Capture {
    pub dir: PathBuf,
    taken: usize,
    /// Frames since the world reported itself built.
    since_built: u32,
    built: bool,
    finished_at: Option<u32>,
    frame: u32,
    /// The probe's intent for `input::gather`, `None` whenever the probe is
    /// only looking (which is every frame of the vantage pass).
    pub intent: Option<Intent>,
    verb: Verb,
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
            intent: None,
            verb: Verb::Hunt,
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
    world: Res<super::WorldId>,
    feed: Res<super::feed::Feed>,
    time: Res<Time>,
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
        // The vantages are shot. Everything past here photographs a VERB,
        // which is the half a fixed camera cannot reach: a swing, and the
        // mark it leaves. `NOW.md` §0ps item 1 — *nobody has looked at
        // either* — is a standing item precisely because the probe could
        // only ever stand still and turn its head.
        verb_pass(
            &mut commands,
            &mut cap,
            &mut look,
            &eye,
            &world,
            &feed,
            time.delta_secs(),
        );
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

/// Face `(dx, dz)` in the sim's own headings.
///
/// A scan of the 256 LUT bearings rather than an `atan2`, and the reason is
/// that the answer has to agree with `sim_core::yaw_dir` — the function the
/// SERVER will resolve this swing's aim cone with. Deriving the angle
/// analytically means re-deriving that LUT's sign and phase conventions from
/// the outside and being quietly one octant out; asking it directly cannot
/// be. This runs once per frame of a probe run and never in a player's
/// client.
fn bearing_to(dx: f32, dz: f32) -> f32 {
    let mut best = (0u16, f32::MIN);
    for hi in 0..=255u16 {
        let (fx, fz) = sim_core::yaw_dir(hi << 8);
        let dot = fx * dx + fz * dz;
        if dot > best.1 {
            best = (hi, dot);
        }
    }
    best.0 as f32 / 256.0 * std::f32::consts::TAU
}

/// Walk at the nearest swingable thing, swing it, and photograph the mark.
///
/// **This is a probe, not a gate.** It writes a frame and scores nothing —
/// `CLAUDE.md` is explicit that the pixel gate was the mistake and a person
/// looking is the visual gate. What this removes is the reason a person
/// could not look: the verb was unreachable without a keyboard.
fn verb_pass(
    commands: &mut Commands,
    cap: &mut Capture,
    look: &mut Look,
    eye: &super::Eye,
    world: &super::WorldId,
    feed: &super::feed::Feed,
    dt: f32,
) {
    match cap.verb {
        Verb::Hunt => {
            // The client resolves the island the same way the server does,
            // so the quarry is found from shared worldgen rather than from
            // anything on the wire — no snapshot carries a tree.
            let (pcx, pcz) = (
                (eye.pos.x / terrain::CELL_SIZE).floor() as i32,
                (eye.pos.z / terrain::CELL_SIZE).floor() as i32,
            );
            let mut best: Option<(f32, f32, f32)> = None;
            for dz in -QUARRY_CELLS..=QUARRY_CELLS {
                for dx in -QUARRY_CELLS..=QUARRY_CELLS {
                    let sl = terrain::scatter(
                        world.seed,
                        &world.table,
                        &world.haven,
                        pcx + dx,
                        pcz + dz,
                    );
                    if sim_core::gather::node_index(sl.occupant).is_none() {
                        continue;
                    }
                    // Skip anything with no skin: a bush is swingable and
                    // leaves no mark by design (`gather::skin_point`), so
                    // walking at one would photograph the absence of the
                    // thing this pass exists to show.
                    if terrain::occupant_volume(sl.occupant).0 <= 0.0 {
                        continue;
                    }
                    let d2 = (sl.x - eye.pos.x).powi(2) + (sl.z - eye.pos.z).powi(2);
                    if best.is_none_or(|b| d2 < b.2) {
                        best = Some((sl.x, sl.z, d2));
                    }
                }
            }
            match best {
                Some((x, z, d2)) => {
                    println!(
                        "capture: quarry at {x:.1},{z:.1} — {:.1} m off, walking",
                        d2.sqrt()
                    );
                    cap.verb = Verb::Walk {
                        x,
                        z,
                        since: cap.frame,
                        best: f32::MAX,
                        improved: cap.frame,
                    };
                }
                None => {
                    // Loud, never silent: an absent frame with no line
                    // printed is indistinguishable from a broken renderer,
                    // which is this repo's worst bug class.
                    eprintln!(
                        "capture: SKIPPED the verb pass — no swingable node with a \
                         surface within {QUARRY_CELLS} cells of {:.0},{:.0}. The \
                         vantages stand; nothing was swung.",
                        eye.pos.x, eye.pos.z
                    );
                    cap.verb = Verb::Done;
                    cap.finished_at = Some(cap.frame);
                }
            }
        }
        Verb::Walk {
            x,
            z,
            since,
            best,
            improved,
        } => {
            let (dx, dz) = (x - eye.pos.x, z - eye.pos.z);
            look.yaw = bearing_to(dx, dz);
            look.pitch = 0.0;
            let d = (dx * dx + dz * dz).sqrt();

            // Arrived, by either of two tests, and the second is the one
            // that actually fires. **In reach** is the server's own
            // question — `gather::swing` measures to the slot CENTRE
            // against `REACH_M` — so anything inside it can be swung at.
            // **Stalled** is how we know we are as close as collision will
            // allow: the body has stopped closing, which for a big rock
            // happens well before any threshold worth picking.
            let stalled = cap.frame - improved > STALL_FRAMES;
            if d <= sim_core::gather::REACH_M * 0.98 || (stalled && d <= sim_core::gather::REACH_M)
            {
                cap.intent = Some(Intent::default());
                cap.verb = Verb::Swing { since: cap.frame };
            } else if stalled {
                eprintln!(
                    "capture: SKIPPED the verb pass — the walk stalled {d:.1} m from \
                     the quarry, past the sim's {:.1} m reach. Something is between \
                     the probe and it, or the quarry is bigger than the reach.",
                    sim_core::gather::REACH_M
                );
                cap.intent = None;
                cap.verb = Verb::Done;
                cap.finished_at = Some(cap.frame);
            } else if cap.frame - since > WALK_FRAMES {
                eprintln!(
                    "capture: SKIPPED the verb pass — {WALK_FRAMES} frames of walking \
                     did not reach the quarry (still {d:.1} m off, closest {best:.1} m)."
                );
                cap.intent = None;
                cap.verb = Verb::Done;
                cap.finished_at = Some(cap.frame);
            } else {
                // Throttle so one frame's travel lands ON the target rather
                // than past it. `dt` is the probe's own frame, which under a
                // CPU rasterizer is about a second, and the movement axis is
                // ANALOG — the sim divides it by 127 — so the correct
                // request is "cover this much ground", not "hold W". At full
                // throttle a frame here is ~3 m and the first cut of this
                // pass oscillated past a 2.3 m quarry forever.
                let full = sim_core::movement::WALK_SPEED * dt.max(1.0 / 240.0);
                let want = (d - sim_core::gather::REACH_M * 0.9) / full;
                let throttle = want.clamp(0.10, 1.0);
                cap.intent = Some(Intent {
                    move_x: 0,
                    move_z: (throttle * 127.0) as i8,
                    buttons: 0,
                });
                // Closing counts as progress only if it is more than the
                // quantization noise of a 3 cm position quantum.
                let (best, improved) = if d < best - 0.05 {
                    (d, cap.frame)
                } else {
                    (best.min(d), improved)
                };
                cap.verb = Verb::Walk {
                    x,
                    z,
                    since,
                    best,
                    improved,
                };
            }
        }
        Verb::Swing { since } => {
            cap.intent = Some(Intent {
                move_x: 0,
                move_z: 0,
                buttons: sim_core::input::BTN_PRIMARY,
            });
            // **Stop on the mark, not on a clock.** `EV_IMPACT` arriving is
            // the sim telling us a swing bit something, which is the exact
            // moment there is a decal worth photographing — and the moment
            // after which every further swing is spent destroying the
            // surface it is drawn on.
            if !feed.impacts().is_empty() {
                cap.intent = Some(Intent::default());
                cap.verb = Verb::Settle { since: cap.frame };
            } else if cap.frame - since > SWING_FRAMES {
                eprintln!(
                    "capture: swung for {SWING_FRAMES} frames and heard no impact — \
                     shooting anyway, but this frame is evidence of nothing."
                );
                cap.intent = Some(Intent::default());
                cap.verb = Verb::Settle { since: cap.frame };
            }
        }
        Verb::Settle { since } => {
            cap.intent = Some(Intent::default());
            if cap.frame - since > MARK_SETTLE_FRAMES {
                let path = cap.dir.join(format!("{}-swing.png", VANTAGES.len()));
                println!("capture: {}", path.display());
                commands
                    .spawn(Screenshot::primary_window())
                    .observe(save_to_disk(path));
                cap.taken += 1;
                cap.intent = None;
                cap.verb = Verb::Done;
                cap.finished_at = Some(cap.frame);
            }
        }
        Verb::Done => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The probe must face what it walks at, through the conversion the
    /// wire actually applies.** `bearing_to` picks a heading off
    /// `sim_core::yaw_dir`'s own LUT and then hands it back as radians for
    /// `Look`, which `look::yaw_u16` re-quantizes on the way to
    /// `set_input`. That round trip is where an off-by-one-octant lives, and
    /// it would show up as a probe that walks past its quarry rather than as
    /// a compile error. Asserted against the LUT's own resolution: 256
    /// headings is 1.4° apart, so the worst honest miss is 0.7°, whose
    /// cosine is 0.99992.
    #[test]
    fn the_probe_faces_what_it_walks_at() {
        for (dx, dz) in [
            (1.0f32, 0.0f32),
            (-1.0, 0.0),
            (0.0, 1.0),
            (0.0, -1.0),
            (0.7, 0.7),
            (-3.0, 1.5),
        ] {
            let yaw = bearing_to(dx, dz);
            let (fx, fz) = sim_core::yaw_dir(crate::look::yaw_u16(yaw));
            let len = (dx * dx + dz * dz).sqrt();
            let dot = fx * dx / len + fz * dz / len;
            assert!(
                dot > 0.999,
                "bearing_to({dx}, {dz}) -> {yaw} rad faces ({fx}, {fz}), dot {dot}"
            );
        }
    }

    /// A fresh probe is only looking, so a player's input path is untouched
    /// by the existence of this module — `input::gather` reads `None` and
    /// falls through to the keyboard.
    #[test]
    fn a_fresh_probe_drives_nothing() {
        let cap = Capture::new(PathBuf::from("/nonexistent"));
        assert!(cap.intent.is_none());
        assert!(cap.verb == Verb::Hunt);
    }
}
