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

/// How many frames the probe may spend walking out of a base before it gives
/// up and shoots anyway. Under lavapipe a frame is about a second and a body
/// covers ~3 m of one, so this is a generous few base-widths.
const CLEAR_FRAMES: u32 = 20;
/// How far the probe looks for pieces when working out which way is out.
const CLEAR_LOOK_M: f32 = 24.0;
/// How far the probe wants to be from the nearest piece before it shoots the
/// fixed vantages, metres.
///
/// **Standing on a base is not the same as being inside one, and the frames
/// needed both rules.** `collide::plane_blocked` answers the second: it
/// short-circuits the moment your feet are within `STEP_UP` of the floor,
/// which is exactly the case where you are standing ON the plate rather than
/// in it. That is a legitimate place for a body to be and a useless place for
/// a camera — six fixed bearings from the middle of somebody's floor are six
/// pictures of the inside of a wall, which is what the operator was looking
/// at. Measured on the shot that prompted this: the probe stood on the plate
/// with a wall 0.4 m away, the near plane at 0.1 m, and nothing clipping —
/// the frame was wrong rather than broken.
///
/// Two thirds of [`BUILD_STANDOFF_M`], which is the range the scene pass
/// already decided a base reads well at; the vantages want less because they
/// are landscape frames that happen to have a base in them rather than
/// portraits of one.
const CLEAR_STANDOFF_M: f32 = BUILD_STANDOFF_M * 2.0 / 3.0;
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

/// Frames the scene pass may wait for a subject to turn up before it gives
/// up and says so.
///
/// A frame count for the reason every other budget here is one, and generous
/// for a reason of its own: **the subjects are produced by another process.**
/// A body and a wall both arrive over the wire from a shard's `population`
/// bots, which dial in, spawn, walk and build on their own schedule — so this
/// waits on somebody else's progress rather than on ours. Under lavapipe a
/// frame is about a second, so this is a minute and a half: long enough for a
/// twig foundation to go up beside the probe, short enough that a shard
/// seated with nobody at all reports an empty island instead of hanging.
pub const SUBJECT_FRAMES: u32 = 90;
/// Frames to hold a subject in the middle of the view before the shutter, so
/// the aim has been where it is for a frame and the body's animation has
/// ticked. The vantages' [`FRAMES_PER_SHOT`] reasoning, for a moving target.
pub const SUBJECT_SETTLE_FRAMES: u32 = 4;
/// How far back the probe wants to stand from a base before photographing it.
///
/// **A wall at arm's length is a texture swatch, not a picture of a
/// building**, and the probe spawns where the builders spawn: `dev_spawn`
/// puts every body on one point, which is exactly what makes a base reachable
/// and exactly what puts the camera inside it. Nine metres frames a
/// foundation and the two levels above it at this rig's field of view.
///
/// It is a WANT, never a requirement — the pass shoots from wherever it
/// actually got to and prints the distance, because a frame taken at 3 m is
/// evidence about a wall and a frame not taken is evidence about nothing.
pub const BUILD_STANDOFF_M: f32 = 9.0;
/// Furthest a body may be and still be worth walking to.
///
/// A bound with a reason rather than a taste: past this a standing figure is
/// a few pixels, and every metre of it is more scatter for the sight test
/// below to walk. Sixty metres is well inside AOI, so the candidates are
/// bodies the server is still describing.
pub const SUBJECT_MAX_M: f32 = 60.0;
/// Metres a tree trunk may sit from the line of sight before the canopy is
/// assumed to be across it.
///
/// **The trunk is the handle; the canopy is what actually hides a body.**
/// `terrain::occupant_volume` gives a tree a 0.2398 m radius, which is the
/// bark — a sight test against that number passes cleanly through a tree
/// whose foliage fills the frame, which is exactly the picture the first
/// three runs of this pass produced. A canopy is centred on its trunk, so
/// trunk-to-line distance is the cheap proxy for it, and two metres is a
/// conifer's drawn spread at the height a body stands.
pub const CANOPY_CLEAR_M: f32 = 2.0;
/// How close the probe wants to be to another player before photographing
/// them.
///
/// **The base is what blocks this shot**, which is the irony of the rig: the
/// bots build a sprawl of twig foundations around the point everybody spawned
/// on, and then the camera cannot see anyone through it. Measured — a body
/// reported 9.9 m away, and again at 25.9 m, both behind a wall the probe was
/// standing inside. Walking at the subject the way the verb pass walks at a
/// quarry puts the camera round the wall more often than not, and six metres
/// frames a standing body head to foot.
pub const PORTRAIT_M: f32 = 6.0;
/// Radius around the nearest piece whose pieces are averaged into the aim
/// point, so the camera frames a BASE rather than one plank of it.
///
/// Two bots build two bases and the mean of all pieces on the island is the
/// empty ground between them — which is the shot this constant exists to
/// refuse. Twelve metres is three building cells: one base, not two.
pub const BUILD_CLUSTER_M: f32 = 12.0;
/// Frames the probe may spend walking to a subject's framing distance.
/// [`WALK_FRAMES`]'s reasoning at a third of the distance.
pub const RANGE_FRAMES: u32 = 60;
/// Conditional shots the tail verifies beside the vantages.
///
/// The swing and the two scene subjects. Each one is *conditional* — an
/// island with no quarry, no neighbour or no base honestly skips it — so
/// these cannot join [`VANTAGES`] in the required list, and a shot that was
/// spawned still has to be proven to have reached disk for the reason the
/// tail check exists at all.
pub const EXTRA_SHOTS: usize = 3;

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
    /// Swung; holding the camera on the point the sim said was struck,
    /// then shooting it.
    ///
    /// **The aim point is the impact's own coordinates**, dequantized from
    /// the wire — not the quarry's centre and not a guess. `render::decal`
    /// places the mark from exactly these numbers, so a camera pointed here
    /// is pointed at the mark if there is one, and photographs its absence
    /// honestly if there is not. Shooting a fixed bearing instead is how
    /// the first cut of this produced a frame of empty grass.
    Settle { since: u32, at: Vec3 },
    /// Over, either shot or honestly skipped.
    Done,
}

/// The two things a probe alone on an island cannot photograph.
#[derive(Clone, Copy, PartialEq)]
enum Subject {
    /// Another player's body, aimed at eye height — the shot a solo capture
    /// can never take, because the only body in the world is the one the
    /// camera is inside.
    Player,
    /// Something somebody built, aimed at the middle of one base.
    Build,
}

impl Subject {
    /// The shot's name on disk, and its index. The numbers continue the
    /// vantages' and the swing's, so a directory listing reads in the order
    /// the frames were taken.
    fn label(self) -> (usize, &'static str) {
        match self {
            Subject::Player => (VANTAGES.len() + 1, "player"),
            Subject::Build => (VANTAGES.len() + 2, "build"),
        }
    }
}

/// How far the probe has got through the scene pass. Ordered, and each arm
/// ends by naming the next.
#[derive(Clone, Copy, PartialEq)]
enum Scene {
    /// Nothing found yet for this subject; scanning every frame since `_`.
    Hunt { subject: Subject, since: u32 },
    /// Walking to `want` metres of `at`, since frame `_`, with the best
    /// distance reached so far and the frame it last improved on.
    ///
    /// One state for both directions, because they are one problem: a body is
    /// usually too FAR to be a portrait and a base is usually too CLOSE to be
    /// a building, and either way the probe wants to stand at a named
    /// distance from a fixed point. **Facing the subject throughout**, so the
    /// frame the shutter reads is the view the walk was aimed by, and so an
    /// obstacle behind the probe shows up as a stall rather than as a camera
    /// swinging round.
    Range {
        subject: Subject,
        at: Vec3,
        want: f32,
        since: u32,
        best: f32,
        improved: u32,
    },
    /// Holding the camera on `at` since frame `_`, shooting when it settles.
    Hold {
        subject: Subject,
        since: u32,
        at: Vec3,
    },
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
    scene: Scene,
    /// Conditional shots that actually reached [`Screenshot`], verified at
    /// the tail beside the vantages. `taken` counts entities SPAWNED, which
    /// is the number the tail check exists because it cannot trust.
    extra: [Option<PathBuf>; EXTRA_SHOTS],
    n_extra: usize,
    /// Has the probe got clear of any base it spawned inside?
    ///
    /// **The vantages are shot from wherever the body is, and `ci/scene.sh`
    /// puts the body where the base goes up.** `dev_spawn` co-locates every
    /// body on one point on purpose — that is what makes the builders and the
    /// camera neighbours rather than kilometres apart — and `--settle` runs
    /// the world for minutes before the probe connects, so the probe arrives
    /// standing in the middle of somebody's floor. Six fixed frames of the
    /// inside of a wall is the result, and it is what the operator saw. An
    /// empty island has no pieces, so this never fires on a plain `--capture`
    /// run and the citable frames stay bit-identical.
    cleared: bool,
    /// Frames spent trying to get clear — the bound, so a probe wedged
    /// somewhere the walk cannot fix shoots its vantages anyway rather than
    /// burning the run. Wall 4's habit applied to a camera.
    clearing: u32,
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
            scene: Scene::Hunt {
                subject: Subject::Player,
                since: 0,
            },
            extra: std::array::from_fn(|_| None),
            n_extra: 0,
            cleared: false,
            clearing: 0,
        }
    }

    /// Take a shot and remember it for the tail check.
    ///
    /// Every shutter in this file goes through here, so `taken` and the
    /// verified set cannot drift apart — the vantages are verified from
    /// [`VANTAGES`] by name and everything else is conditional, so an
    /// unrecorded conditional shot would be one nothing ever proves landed.
    /// Over the cap the shot is still taken and simply unverified, which is
    /// wall 4's bounded-with-a-stated-policy in its mildest form: the cap is
    /// the number of conditional shots this file can take.
    fn shoot(&mut self, commands: &mut Commands, path: PathBuf, extra: bool) {
        println!("capture: {}", path.display());
        if extra {
            if self.n_extra < EXTRA_SHOTS {
                self.extra[self.n_extra] = Some(path.clone());
                self.n_extra += 1;
            } else {
                eprintln!(
                    "capture: over {EXTRA_SHOTS} conditional shots — {} is unverified",
                    path.display()
                );
            }
        }
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(path));
        self.taken += 1;
    }

    /// The verb pass is over; the scene pass begins. Every terminal arm of
    /// [`verb_pass`] ends here rather than at `finished_at`, which is what
    /// keeps the two passes in one order instead of racing.
    fn begin_scene(&mut self) {
        self.scene = Scene::Hunt {
            subject: Subject::Player,
            since: self.frame,
        };
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
    // `Option`, and not because it might legitimately be absent: `WorldId`
    // and `Net` are inserted from one command buffer and land together, so
    // `world_running` implies this. A missing non-send resource is a PANIC
    // rather than a skip, though, and a panicking probe writes no frames and
    // no diagnosis — the same "worst bug class" the tail check below exists
    // for, arriving as a crash instead of an empty directory.
    net: Option<NonSend<super::Net>>,
    screen: Res<State<super::menu::Screen>>,
) {
    cap.frame += 1;
    look.frozen = true;

    // **The probe is a player, and things kill it** (`RENDER.md`: three of
    // four runs carrying a heavy change died where none of two baseline runs
    // did, and it was diagnosed as everything else first). A dead body still
    // streams the world behind the death wash, so `world_running` is true
    // here and every check below would pass — the run would shoot six
    // vantages of a red screen and exit 0. Worse, `input::gather` stands down
    // for a corpse, so the verb and scene passes would spend their whole
    // budgets walking nowhere and then photograph that.
    //
    // Loud, and nonzero: the frames already on disk are real and stay, and
    // the ones that did not happen are named as a death rather than left to
    // be read as a slow build. A shard seated with `population` makes this
    // likelier, not less — a raider's charge does not check who is standing
    // beside the base.
    if *screen.get() == super::menu::Screen::Dead {
        eprintln!(
            "capture: the probe DIED at frame {} — {} frame(s) written, the rest are \
             not coming. Nothing below this line is evidence about a build. Pin \
             `dev_spawn` somewhere the mob roster has not homed on (RENDER.md).",
            cap.frame, cap.taken
        );
        exit.write(AppExit::error());
        return;
    }

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

    // Step out of anything the probe spawned inside, BEFORE the vantage clock
    // starts — `since_built` is what indexes the shots, so this holds it
    // rather than spending it.
    if !cap.cleared && !clear_of_a_base(&mut cap, &mut look, &eye, &world, net.as_deref()) {
        return;
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
            // The conditional shots, by the paths actually spawned. A
            // skipped subject recorded nothing and is therefore not missed —
            // absence is honest here, a spawned shot that never landed is not.
            for path in cap.extra[..cap.n_extra].iter().flatten() {
                match std::fs::metadata(path) {
                    Ok(m) if m.is_file() && m.len() > 0 => {}
                    _ => missing.push(path.clone()),
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
                    "capture: {} of {} frames did not reach disk",
                    missing.len(),
                    VANTAGES.len() + cap.n_extra
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
        // Two passes, in order, and the order is the reason the scene pass
        // runs second. The verb pass WALKS — up to `QUARRY_CELLS` of cells
        // away, at a rock — so by the time it is done the probe is standing
        // back from the spot every body spawned on, which is the standoff the
        // scene pass would otherwise have to walk for.
        if cap.verb != Verb::Done {
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
        scene_pass(
            &mut commands,
            &mut cap,
            &mut look,
            &eye,
            &world,
            net.as_deref(),
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
        cap.shoot(&mut commands, path, false);
    }
}

/// Walk the probe out of any base it is standing inside. `true` once it is
/// clear (or has given up), `false` while it is still walking.
///
/// **The test is the sim's own** — `collide::plane_blocked` against the
/// predictor's mirror of the piece store — rather than a distance to the
/// nearest record. A body is "inside" exactly when the thing that stops
/// bodies says so, and reproducing that judgement here would be the
/// hand-kept mirror this repo keeps paying for. It is also why this could
/// not be written before piece flanks v0: until a plane had sides, there was
/// no function to ask.
///
/// The direction is away from the CENTROID of the nearby pieces rather than
/// away from the nearest one: the nearest piece of a base you are standing in
/// the middle of is behind you as often as in front, and a probe walking away
/// from it would cross the base rather than leave it.
fn clear_of_a_base(
    cap: &mut Capture,
    look: &mut Look,
    eye: &super::Eye,
    world: &super::WorldId,
    net: Option<&super::Net>,
) -> bool {
    let Some(net) = net else { return true };
    let core = &net.session.core;
    let feet = eye.pos.y - super::EYE_HEIGHT;
    let inside = sim_core::collide::plane_blocked(
        world.seed,
        &world.haven,
        core.pieces.cols(),
        eye.pos.x,
        eye.pos.z,
        feet,
    );
    let nearest = core
        .pieces
        .entries()
        .iter()
        .map(|rec| {
            let (ax, az) = sim_core::build::anchor(rec.cx, rec.cz, rec.loc);
            let (dx, dz) = (ax - eye.pos.x, az - eye.pos.z);
            dx * dx + dz * dz
        })
        .fold(f32::INFINITY, f32::min);
    if !inside && nearest >= CLEAR_STANDOFF_M * CLEAR_STANDOFF_M {
        if cap.clearing > 0 {
            println!(
                "capture: walked clear of a base in {} frame(s) before the vantages",
                cap.clearing
            );
        }
        cap.cleared = true;
        cap.intent = None;
        return true;
    }
    cap.clearing += 1;
    if cap.clearing > CLEAR_FRAMES {
        eprintln!(
            "capture: still inside a base after {CLEAR_FRAMES} frames — shooting the \
             vantages from here, so they will show the inside of a wall."
        );
        cap.cleared = true;
        cap.intent = None;
        return true;
    }
    let (mut sx, mut sz, mut n) = (0.0f32, 0.0f32, 0.0f32);
    for rec in core.pieces.entries() {
        let (ax, az) = sim_core::build::anchor(rec.cx, rec.cz, rec.loc);
        let (dx, dz) = (ax - eye.pos.x, az - eye.pos.z);
        if dx * dx + dz * dz <= CLEAR_LOOK_M * CLEAR_LOOK_M {
            sx += ax;
            sz += az;
            n += 1.0;
        }
    }
    // Nothing near enough to walk away from, and yet `plane_blocked` says
    // inside: hold still rather than sprinting at a bearing computed from
    // nothing. The frame budget above is what ends this case.
    if n == 0.0 {
        return false;
    }
    let (dx, dz) = (eye.pos.x - sx / n, eye.pos.z - sz / n);
    look.yaw = bearing_to(dx, dz);
    look.pitch = 0.0;
    cap.intent = Some(Intent {
        move_x: 0,
        move_z: 127,
        buttons: 0,
    });
    false
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
                    cap.begin_scene();
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
                cap.begin_scene();
            } else if cap.frame - since > WALK_FRAMES {
                eprintln!(
                    "capture: SKIPPED the verb pass — {WALK_FRAMES} frames of walking \
                     did not reach the quarry (still {d:.1} m off, closest {best:.1} m)."
                );
                cap.intent = None;
                cap.verb = Verb::Done;
                cap.begin_scene();
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
            if let Some(&(qx, qy, qz, surf)) = feed.impacts().first() {
                let at = Vec3::new(
                    qx as f32 * sim_core::movement::POS_XZ_Q,
                    qy as f32 * sim_core::movement::POS_Y_Q,
                    qz as f32 * sim_core::movement::POS_XZ_Q,
                );
                println!(
                    "capture: mark at {:.2},{:.2},{:.2} (surf {surf}) — eye at \
                     {:.2},{:.2},{:.2}, {:.2} m away",
                    at.x,
                    at.y,
                    at.z,
                    eye.pos.x,
                    eye.pos.y,
                    eye.pos.z,
                    at.distance(eye.pos)
                );
                cap.intent = Some(Intent::default());
                cap.verb = Verb::Settle {
                    since: cap.frame,
                    at,
                };
            } else if cap.frame - since > SWING_FRAMES {
                eprintln!(
                    "capture: swung for {SWING_FRAMES} frames and heard no impact — \
                     shooting anyway, but this frame is evidence of nothing."
                );
                cap.intent = Some(Intent::default());
                cap.verb = Verb::Settle {
                    since: cap.frame,
                    at: eye.pos,
                };
            }
        }
        Verb::Settle { since, at } => {
            cap.intent = Some(Intent::default());
            // Point the camera AT the mark. Melee ignores pitch, so aiming
            // down here cannot change what the sim would have done — the
            // swing is already spent.
            let (dx, dy, dz) = (at.x - eye.pos.x, at.y - eye.pos.y, at.z - eye.pos.z);
            let flat = (dx * dx + dz * dz).sqrt();
            if flat > 0.01 {
                look.yaw = bearing_to(dx, dz);
                look.pitch = dy.atan2(flat);
            }
            if cap.frame - since > MARK_SETTLE_FRAMES {
                let path = cap.dir.join(format!("{}-swing.png", VANTAGES.len()));
                cap.shoot(commands, path, true);
                cap.intent = None;
                cap.verb = Verb::Done;
                cap.begin_scene();
            }
        }
        Verb::Done => {}
    }
}

/// Point the camera at another player, and at something somebody built.
///
/// **This is the half a probe alone on an island cannot reach.** Every frame
/// the vantage pass takes is a photograph of terrain, because terrain is the
/// only thing in the world when one client is the whole population — and the
/// two questions anybody actually asks of a build ("what does another player
/// look like from here", "do the pieces line up") need a subject this process
/// does not produce. The subject comes from a shard seated with `population`:
/// bots that dial the real wire, spawn where `dev_spawn` puts everyone, and
/// build. `ci/scene.sh` is the rig that arranges both halves.
///
/// **It photographs what is there and says so when nothing is**, which is the
/// verb pass's posture and for the verb pass's reason: an absent frame with
/// no line printed is indistinguishable from a broken renderer.
fn scene_pass(
    commands: &mut Commands,
    cap: &mut Capture,
    look: &mut Look,
    eye: &super::Eye,
    world: &super::WorldId,
    net: Option<&super::Net>,
    dt: f32,
) {
    let Some(net) = net else {
        // Unreachable by the argument at the call site, and handled rather
        // than asserted anyway: this is the one path that could end a run
        // with no frames and no reason printed.
        if !matches!(cap.scene, Scene::Done) {
            eprintln!("capture: SKIPPED the scene pass — no session to read bodies from.");
            cap.scene = Scene::Done;
            cap.finished_at = Some(cap.frame);
        }
        return;
    };
    let core = &net.session.core;

    match cap.scene {
        Scene::Hunt { subject, since } => {
            let found = match subject {
                // The interpolator, not the predictor: a remote body is
                // somebody else's input and predicting it would draw the
                // camera at a place the server never put them (`bodies.rs`).
                Subject::Player => {
                    let at = core.render_tick();
                    let mut rs = client_core::interp::RemoteState::default();
                    // Nearest body with nothing across the line of sight,
                    // and — as the fallback — nearest body at all. Ranked
                    // rather than filtered for the reason `sight_is_clear`
                    // gives: a clear answer is a heuristic and a blocked one
                    // is only about trees, so refusing to shoot on it would
                    // trade a shot of a body behind a canopy for no shot.
                    let mut best: Option<(Vec3, f32)> = None;
                    let mut any: Option<(Vec3, f32)> = None;
                    let mut stale = 0usize;
                    for id in core.interp.ids() {
                        if id == core.player_id {
                            continue;
                        }
                        // **Animals are on this lane too, and they are not
                        // people.** `bodies.rs` carries the long version and
                        // the scar: an animal is the same class-D record a
                        // player is, separated only by the high bit of the id,
                        // and the last thing to forget this drew a humanoid
                        // rig standing inside every pig. This pass forgot it
                        // too — the first run of it, against a shard with a
                        // population of ZERO, reported "player at 67.3 m" and
                        // photographed a wolf. `mobs::stream` takes the half
                        // this skips and the two are exact complements.
                        if sim_core::mob::slot_of_id(id).is_some() {
                            continue;
                        }
                        if !core.interp.sample(id, at, &mut rs) {
                            continue;
                        }
                        // Eye height, so the shot is one player looking at
                        // another rather than at their boots. `rs` is feet,
                        // `eye.pos` is already `+ EYE_HEIGHT` (`input.rs`).
                        let p = Vec3::new(rs.x, rs.y + super::EYE_HEIGHT, rs.z);
                        let d2 = (p.x - eye.pos.x).powi(2) + (p.z - eye.pos.z).powi(2);
                        // Too far to be a picture of anybody. Bounded here
                        // rather than after the scan, so the sight test below
                        // never walks a 400 m line of cells.
                        if d2 > SUBJECT_MAX_M * SUBJECT_MAX_M {
                            continue;
                        }
                        // **`live` is recorded, not obeyed, and the reason is
                        // that this camera has to agree with the RENDERER
                        // rather than with the server.** A body whose updates
                        // stopped — walked out of AOI, shift ended — samples
                        // as a clamp at its last known place, and
                        // `bodies::stream` reads the same interpolator with no
                        // `live` check at all, so it draws a mannequin at
                        // exactly that clamp. Aiming anywhere else would miss
                        // a body that is on screen.
                        if !rs.live {
                            stale += 1;
                        }
                        if any.is_none_or(|b| d2 < b.1) {
                            any = Some((p, d2));
                        }
                        if sight_is_clear(world, core, eye.pos, p) && best.is_none_or(|b| d2 < b.1)
                        {
                            best = Some((p, d2));
                        }
                    }
                    // Falling back is loud, because the frame that comes
                    // out of it is a different kind of evidence: the camera
                    // is pointed at a body that a tree is probably standing
                    // in front of, which is a photograph of the tree.
                    if best.is_none() && any.is_some() {
                        eprintln!(
                            "capture: every body in range has a tree or a wall across it — \
                             shooting the nearest anyway. The aim is right; expect something \
                             in front of it."
                        );
                    }
                    if stale > 0 && best.or(any).is_some() {
                        println!(
                            "capture: {stale} of the bodies in range are clamps (their updates \
                             stopped); they are drawn where they are aimed."
                        );
                    }
                    best.or(any)
                }
                // The piece mirror is the renderer's truth (`core.rs`), so
                // this frames what is actually drawn rather than what the
                // sim would draw if the defs had arrived.
                Subject::Build => {
                    let mut near: Option<(Vec3, f32)> = None;
                    for rec in core.pieces.entries() {
                        let t = super::structures::base_transform(
                            world.seed,
                            &world.haven,
                            (rec.cx, rec.cz, rec.level, rec.loc),
                            rec.plate,
                        );
                        let p = t.translation;
                        let d2 = (p.x - eye.pos.x).powi(2) + (p.z - eye.pos.z).powi(2);
                        if near.is_none_or(|b| d2 < b.1) {
                            near = Some((p, d2));
                        }
                    }
                    // Average the pieces around the nearest one, so the aim
                    // is the middle of ONE base. Averaging every piece on the
                    // island puts the camera on the empty ground between two
                    // of them, which is the shot `BUILD_CLUSTER_M` refuses.
                    near.map(|(seed_piece, _)| {
                        let (mut sum, mut n) = (Vec3::ZERO, 0.0f32);
                        for rec in core.pieces.entries() {
                            let t = super::structures::base_transform(
                                world.seed,
                                &world.haven,
                                (rec.cx, rec.cz, rec.level, rec.loc),
                                rec.plate,
                            );
                            if t.translation.distance(seed_piece) <= BUILD_CLUSTER_M {
                                sum += t.translation;
                                n += 1.0;
                            }
                        }
                        // A metre up off the centroid: a lone foundation's
                        // centre is the ground, and a camera pointed at the
                        // ground photographs the dirt in front of it.
                        let c = sum / n.max(1.0) + Vec3::Y;
                        let d2 = (c.x - eye.pos.x).powi(2) + (c.z - eye.pos.z).powi(2);
                        (c, d2)
                    })
                }
            };

            match found {
                Some((at, d2)) => {
                    let d = d2.sqrt();
                    let (_, label) = subject.label();
                    println!(
                        "capture: {label} at {:.1},{:.1},{:.1} — {d:.1} m off",
                        at.x, at.y, at.z
                    );
                    // Each subject has a distance it reads well at, and
                    // they miss it in opposite directions: a base is usually
                    // too close (the probe spawned inside it) and a body is
                    // usually too far and behind a wall of it. Walk either
                    // way; already at the right range is the third case and
                    // needs no walk at all.
                    let want = match subject {
                        Subject::Player => PORTRAIT_M,
                        Subject::Build => BUILD_STANDOFF_M,
                    };
                    cap.scene = if (d - want).abs() > 1.0 {
                        Scene::Range {
                            subject,
                            at,
                            want,
                            since: cap.frame,
                            best: (d - want).abs(),
                            improved: cap.frame,
                        }
                    } else {
                        Scene::Hold {
                            subject,
                            since: cap.frame,
                            at,
                        }
                    };
                }
                None if cap.frame - since > SUBJECT_FRAMES => {
                    let (_, label) = subject.label();
                    eprintln!(
                        "capture: SKIPPED the {label} shot — {SUBJECT_FRAMES} frames and no \
                         {} in the world. Seat a shard with `population` (see ci/scene.sh); \
                         a probe alone on an island has nothing to photograph.",
                        match subject {
                            Subject::Player => "other body",
                            Subject::Build => "placed piece",
                        }
                    );
                    advance(cap, subject);
                }
                None => {}
            }
        }

        Scene::Range {
            subject,
            at,
            want,
            since,
            best,
            improved,
        } => {
            // Face the subject the whole way, whichever direction the feet
            // are going: the frame the shutter reads is then the view the walk
            // was aimed by, and an obstacle reads as a stall rather than as a
            // camera swinging round to look at it.
            let (dx, dz) = (at.x - eye.pos.x, at.z - eye.pos.z);
            look.yaw = bearing_to(dx, dz);
            look.pitch = 0.0;
            let d = (dx * dx + dz * dz).sqrt();
            let gap = d - want;
            let stalled = cap.frame - improved > STALL_FRAMES;
            let out_of_frames = cap.frame - since > RANGE_FRAMES;

            if gap.abs() <= 0.5 || stalled || out_of_frames {
                if gap.abs() > 1.0 {
                    // Loud, and then shoot anyway: a frame taken from the
                    // wrong distance is evidence about the thing, and a frame
                    // not taken is evidence about nothing. Which of the two
                    // stopped the walk matters — "something is in the way" and
                    // "this was always going to take longer" are different
                    // reads of the same distance.
                    let (_, label) = subject.label();
                    eprintln!(
                        "capture: stopped {d:.1} m from the {label}, wanting {want:.0} m ({}). \
                         Shooting from here.",
                        if stalled {
                            "something is in the way"
                        } else {
                            "out of frames"
                        }
                    );
                }
                cap.intent = Some(Intent::default());
                cap.scene = Scene::Hold {
                    subject,
                    since: cap.frame,
                    at,
                };
            } else {
                // `Verb::Walk`'s throttle and its measured reason: the axis is
                // ANALOG, the sim divides it by 127, and one probe frame under
                // lavapipe is ~3 m at full throttle — so the request is "cover
                // this much ground", not "hold W". The first cut of the verb
                // pass held W and oscillated past a 2.3 m quarry forever.
                // `gap` is signed, so backing away from a base and closing on
                // a body are the same line of arithmetic.
                let full = sim_core::movement::WALK_SPEED * dt.max(1.0 / 240.0);
                let throttle = (gap.abs() / full).clamp(0.10, 1.0) * gap.signum();
                cap.intent = Some(Intent {
                    move_x: 0,
                    move_z: (throttle * 127.0) as i8,
                    buttons: 0,
                });
                // Progress is measured against the GAP, not the distance:
                // closing and backing away both count, and neither reads as a
                // stall while it is still happening.
                let (best, improved) = if gap.abs() < best - 0.05 {
                    (gap.abs(), cap.frame)
                } else {
                    (best.min(gap.abs()), improved)
                };
                cap.scene = Scene::Range {
                    subject,
                    at,
                    want,
                    since,
                    best,
                    improved,
                };
            }
        }

        Scene::Hold { subject, since, at } => {
            cap.intent = Some(Intent::default());
            let (dx, dy, dz) = (at.x - eye.pos.x, at.y - eye.pos.y, at.z - eye.pos.z);
            let flat = (dx * dx + dz * dz).sqrt();
            if flat > 0.01 {
                look.yaw = bearing_to(dx, dz);
                look.pitch = dy.atan2(flat);
            }
            if cap.frame - since > SUBJECT_SETTLE_FRAMES {
                let (idx, label) = subject.label();
                let path = cap.dir.join(format!("{idx}-{label}.png"));
                cap.shoot(commands, path, true);
                advance(cap, subject);
            }
        }

        Scene::Done => {}
    }
}

/// Squared distance from `(px, pz)` to the segment `(ax, az) → (bx, bz)`, in
/// the ground plane.
///
/// Flat on purpose: what hides a standing body is a trunk beside the line,
/// and a canopy's height is not what decides that. Clamped to the segment, so
/// a tree BEHIND the camera or beyond the subject is not counted as being in
/// the way — the unclamped form (distance to the infinite line) rejects half
/// the island.
fn seg_dist2(px: f32, pz: f32, ax: f32, az: f32, bx: f32, bz: f32) -> f32 {
    let (vx, vz) = (bx - ax, bz - az);
    let (wx, wz) = (px - ax, pz - az);
    let len2 = vx * vx + vz * vz;
    let t = if len2 > 0.0 {
        ((wx * vx + wz * vz) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let (dx, dz) = (wx - vx * t, wz - vz * t);
    dx * dx + dz * dz
}

/// Would a body at `to` be visible from `from`, or is the world in the way?
///
/// **Two blockers, from two different places, and neither is a real
/// raycast.** Trees come from shared worldgen — no snapshot carries one, so
/// the client resolves them exactly as the verb pass resolves its quarry —
/// and walls come from the piece mirror the renderer draws from. Both are
/// tested the same cheap way: distance from the thing's centre to the sight
/// SEGMENT, against a radius.
///
/// **Walls, not floors**, and `loc` is what separates them: `LOC_PLANE` is
/// the horizontal piece a body STANDS on, so counting it would reject
/// everybody in the base as hidden by the ground under their feet. An edge
/// address is a vertical piece, and that is the one across the view.
///
/// It stays a heuristic and says so: a clear answer means "no trunk and no
/// wall near the line", never "you will see them". The caller ranks with it
/// and falls back loudly rather than refusing to shoot on it.
fn sight_is_clear(
    world: &super::WorldId,
    core: &client_core::core::ClientCore,
    from: Vec3,
    to: Vec3,
) -> bool {
    for rec in core.pieces.entries() {
        if rec.loc == sim_core::build::LOC_PLANE {
            continue;
        }
        let t = super::structures::base_transform(
            world.seed,
            &world.haven,
            (rec.cx, rec.cz, rec.level, rec.loc),
            rec.plate,
        );
        let c = t.translation;
        // Vertically out of the way: a wall two storeys up is not between
        // two people standing on the ground. The eye and the subject are
        // both around 1.6 m, so a span either side of the line's height
        // covers what could be across it.
        let span = super::structures::piece_span();
        if (c.y - (from.y + to.y) * 0.5).abs() > span {
            continue;
        }
        let r = span * 0.5;
        if seg_dist2(c.x, c.z, from.x, from.z, to.x, to.z) < r * r {
            return false;
        }
    }
    let pad = CANOPY_CLEAR_M + terrain::CELL_SIZE;
    let cell = |v: f32| (v / terrain::CELL_SIZE).floor() as i32;
    let (cx0, cx1) = (cell(from.x.min(to.x) - pad), cell(from.x.max(to.x) + pad));
    let (cz0, cz1) = (cell(from.z.min(to.z) - pad), cell(from.z.max(to.z) + pad));
    for cz in cz0..=cz1 {
        for cx in cx0..=cx1 {
            let sl = terrain::scatter(world.seed, &world.table, &world.haven, cx, cz);
            let (r, h) = terrain::occupant_volume(sl.occupant);
            // Only things tall enough to hide a standing body. A boulder is
            // 1.5 m and the camera is at 1.6 m, so it is scenery in front of
            // somebody rather than a wall across them.
            if h < 2.0 {
                continue;
            }
            let clear = r.max(CANOPY_CLEAR_M);
            if seg_dist2(sl.x, sl.z, from.x, from.z, to.x, to.z) < clear * clear {
                return false;
            }
        }
    }
    true
}

/// Move the scene pass on to the next subject, or end the run.
///
/// One place, so a shot and a skip leave the pass in the same state — the
/// four call sites above are two of each, and a skip that forgot to advance
/// would hang the probe on a subject it had already given up on.
fn advance(cap: &mut Capture, done: Subject) {
    match done {
        Subject::Player => {
            cap.scene = Scene::Hunt {
                subject: Subject::Build,
                since: cap.frame,
            }
        }
        Subject::Build => {
            cap.intent = None;
            cap.scene = Scene::Done;
            cap.finished_at = Some(cap.frame);
        }
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

    /// **No two shots may claim one filename**, and the failure would be
    /// silent: the second `save_to_disk` overwrites the first, the tail check
    /// finds a non-empty file at every path it asks about, and the run
    /// reports a full set having thrown a frame away. The swing is
    /// `VANTAGES.len()` and the subjects continue from it, so adding a
    /// vantage moves all three together — which is the property this asserts
    /// rather than the three numbers themselves.
    #[test]
    fn every_shot_has_its_own_name() {
        let mut names: Vec<String> = (0..VANTAGES.len())
            .map(|i| format!("{i}-{}", VANTAGES[i].1))
            .collect();
        names.push(format!("{}-swing", VANTAGES.len()));
        for s in [Subject::Player, Subject::Build] {
            let (idx, label) = s.label();
            names.push(format!("{idx}-{label}"));
        }
        let n = names.len();
        names.sort();
        names.dedup();
        assert_eq!(n, names.len(), "two shots share a filename: {names:?}");

        // Indices are dense from zero, so a listing reads in shooting order
        // and a gap means a shot lost its number.
        let mut idx: Vec<usize> = (0..VANTAGES.len()).collect();
        idx.push(VANTAGES.len());
        idx.extend([Subject::Player, Subject::Build].map(|s| s.label().0));
        idx.sort();
        assert!(
            idx.iter().enumerate().all(|(i, &n)| i == n),
            "shot indices are not dense from 0: {idx:?}"
        );
    }

    /// **The tail check can only verify what it has room to remember.**
    /// `EXTRA_SHOTS` bounds the conditional shots (wall 4 wants a cap and a
    /// stated policy), and the policy is "shoot it, say it is unverified" —
    /// so a cap set below the number of conditional shots this file actually
    /// takes would be a permanent complaint on every full run. The swing,
    /// plus one per subject.
    #[test]
    fn the_conditional_shots_all_fit() {
        assert_eq!(EXTRA_SHOTS, 1 + [Subject::Player, Subject::Build].len());
    }

    /// **A skipped subject must leave the pass where a shot one does.** Both
    /// call `advance`, and the failure mode if one forgot to is not a missing
    /// frame — it is a probe that hunts a subject it has already given up on
    /// until the process is killed, which writes nothing and explains
    /// nothing. Asserted as the two transitions plus the ending.
    #[test]
    fn every_subject_hands_over_or_ends_the_run() {
        let mut cap = Capture::new(PathBuf::from("/nonexistent"));
        cap.frame = 7;
        advance(&mut cap, Subject::Player);
        assert!(
            matches!(
                cap.scene,
                Scene::Hunt {
                    subject: Subject::Build,
                    since: 7
                }
            ),
            "the player shot did not hand over to the build shot"
        );
        assert!(cap.finished_at.is_none(), "the run ended a subject early");

        advance(&mut cap, Subject::Build);
        assert!(matches!(cap.scene, Scene::Done));
        assert_eq!(
            cap.finished_at,
            Some(7),
            "the last subject did not end the run — the probe would hang"
        );
        assert!(cap.intent.is_none(), "the probe was left driving");
    }

    /// **The sight test is clamped to the SEGMENT, and the unclamped form is
    /// the bug it is written against.** Distance to the infinite line rejects
    /// a tree standing behind the camera or well past the subject, which on a
    /// wooded island is most of them — the pass would then report every body
    /// as blocked and never prefer anything. Asserted at both ends and in the
    /// middle, with the perpendicular case as the one that must still fire.
    #[test]
    fn only_what_is_between_the_two_counts() {
        // Line from the origin to (10, 0).
        let d2 = |px, pz| seg_dist2(px, pz, 0.0, 0.0, 10.0, 0.0);
        // Beside the middle: in the way.
        assert!(d2(5.0, 1.0) - 1.0 < 1e-4, "{}", d2(5.0, 1.0));
        // On the line but BEHIND the camera, and the same distance PAST the
        // subject: both are 5 m away from the segment, not 0.
        assert!(
            (d2(-5.0, 0.0) - 25.0).abs() < 1e-3,
            "behind: {}",
            d2(-5.0, 0.0)
        );
        assert!(
            (d2(15.0, 0.0) - 25.0).abs() < 1e-3,
            "past: {}",
            d2(15.0, 0.0)
        );
        // Degenerate segment (subject at the eye) is a point distance, not a
        // divide by zero.
        assert!((seg_dist2(3.0, 4.0, 1.0, 1.0, 1.0, 1.0) - 13.0).abs() < 1e-3);
    }
}
