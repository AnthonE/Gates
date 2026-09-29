//! Film mode: `gates --film <shots.json>` — the trailer camera (`ci/film.sh`).
//!
//! A recorded session (`crate::film`) drawn on a fixed frame clock with a
//! scripted camera. Each shot is piped into its own `ffmpeg` as it renders,
//! and the game's own mix is written beside it as a WAV. It exists because the
//! renderer a box without a GPU has is lavapipe — about a frame a second — so
//! footage of a live session would be a slideshow of a world at 30x speed.
//!
//! Three clocks, and keeping them apart is the whole design:
//!
//!   · **the frame clock** — a frame is `1/fps` of screen time however long it
//!     took to draw (`TimeUpdateStrategy::ManualDuration`, `bin/gates.rs`);
//!   · **the world clock** — `Time<Virtual>` at the shot's `speed`: what the
//!     replay, the core and every animation advance by. `speed: 8` is a
//!     time-lapse;
//!   · **the shot clock** — screen seconds into the shot, which the camera
//!     path is keyed on, so a time-lapse still pans at an even pace.
//!
//! Bevy still only draws: the replay feeds the session's lanes and the
//! ordinary `pump` drains them, so the world on screen is exactly what a live
//! client would have drawn. The one thing this file writes is the eye.

use std::collections::BTreeMap;
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use serde::Deserialize;
use sim_core::terrain;

use super::clutter::ClutterRing;
use super::props::PropRing;
use super::terrain_mesh::Ring;
use super::{Eye, Net, WorldId};

/// Frames a shot holds its first pose before rolling, by default: the rings
/// re-centre on a camera that has just jumped, and pipelines specialise on
/// first draw.
pub const SETTLE_FRAMES: u32 = 45;
/// The most a settle may run past [`SETTLE_FRAMES`] waiting for the rings
/// before it rolls anyway and says so.
pub const SETTLE_MAX_FRAMES: u32 = 300;
/// The audio rate the film's mix is rendered at: video's, and a whole number
/// of samples per frame at 24, 25, 30 and 60 fps.
pub const FILM_RATE: u32 = 48_000;

fn default_width() -> u32 {
    1920
}
fn default_height() -> u32 {
    1080
}
fn default_fps() -> u32 {
    30
}
fn default_fov() -> f32 {
    55.0
}
fn one() -> f64 {
    1.0
}
fn default_settle() -> u32 {
    SETTLE_FRAMES
}

/// The shot list, as `ci/film.sh` writes it.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Script {
    /// The tape (`bin/record.rs`).
    pub recording: PathBuf,
    /// Where the shots go: `NN-name.mp4` and `NN-name.wav`.
    pub out: PathBuf,
    #[serde(default = "default_width")]
    pub width: u32,
    #[serde(default = "default_height")]
    pub height: u32,
    #[serde(default = "default_fps")]
    pub fps: u32,
    /// Vertical field of view in degrees for shots that do not name one.
    #[serde(default = "default_fov")]
    pub fov: f32,
    /// Frames each shot holds its first pose before rolling.
    #[serde(default = "default_settle")]
    pub settle: u32,
    pub shots: Vec<Shot>,
}

impl Script {
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut s: Script =
            serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        if s.shots.is_empty() {
            return Err(format!("{}: no shots", path.display()));
        }
        if s.fps == 0 || !FILM_RATE.is_multiple_of(s.fps) {
            return Err(format!(
                "fps {} does not divide {FILM_RATE} Hz audio",
                s.fps
            ));
        }
        for shot in &s.shots {
            shot.check()?;
        }
        // The tape only plays forward, so the shots are taken in tape order.
        s.shots.sort_by(|a, b| a.at.total_cmp(&b.at));
        Ok(s)
    }
}

/// One value, or one eased to another across the shot.
#[derive(Deserialize, Clone, Copy, Debug)]
#[serde(untagged)]
pub enum Span {
    At(f32),
    Between([f32; 2]),
}

impl Span {
    fn at(self, u: f32) -> f32 {
        match self {
            Span::At(v) => v,
            Span::Between([a, b]) => a + (b - a) * u,
        }
    }
}

#[derive(Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Shot {
    pub name: String,
    /// Seconds into the tape at the shot's first frame.
    pub at: f64,
    /// Seconds of screen time.
    pub len: f64,
    /// Tape seconds per screen second: 1 is real time, 8 a time-lapse.
    #[serde(default = "one")]
    pub speed: f64,
    /// Day fraction to pin the sky at (0.5 noon, 0.75 dusk); a pair eases
    /// across the shot. Unset is the tape's own hour.
    #[serde(default)]
    pub hour: Option<Span>,
    /// clear | overcast | fog | rain | heavy | storm. Unset is the tape's.
    #[serde(default)]
    pub weather: Option<String>,
    /// Vertical field of view, degrees; a pair is a zoom.
    #[serde(default)]
    pub fov: Option<Span>,
    /// Smoothstep the shot clock, so the move starts and ends at rest.
    #[serde(default)]
    pub ease: bool,
    /// A camera path: at least two keys.
    #[serde(default)]
    pub keys: Vec<Key>,
    #[serde(default)]
    pub orbit: Option<Orbit>,
    #[serde(default)]
    pub follow: Option<Follow>,
    /// Keep a body in frame: the keys move the camera, this aims it.
    #[serde(default)]
    pub track: Option<Track>,
}

impl Shot {
    fn check(&self) -> Result<(), String> {
        let n = &self.name;
        let paths = usize::from(self.keys.len() >= 2)
            + usize::from(self.orbit.is_some())
            + usize::from(self.follow.is_some());
        if paths != 1 {
            return Err(format!(
                "shot `{n}`: give exactly one of keys (two or more), orbit or follow"
            ));
        }
        if self.track.is_some() && self.follow.is_some() {
            return Err(format!(
                "shot `{n}`: track aims a camera keys or an orbit move"
            ));
        }
        if self.len <= 0.0 || self.speed <= 0.0 || self.at < 0.0 {
            return Err(format!("shot `{n}`: at, len and speed must be positive"));
        }
        if let Some(w) = &self.weather {
            weather_code(w).ok_or_else(|| format!("shot `{n}`: unknown weather `{w}`"))?;
        }
        Ok(())
    }
}

/// A camera key. Heights are metres above the ground (or the sea) under the
/// point, so a path can be written from the map without knowing the terrain.
#[derive(Deserialize, Clone, Copy, Debug)]
#[serde(deny_unknown_fields)]
pub struct Key {
    /// Screen seconds into the shot.
    pub t: f32,
    /// `[x, above ground, z]`.
    pub pos: [f32; 3],
    /// What the camera looks at, `[x, above ground, z]`.
    pub look: [f32; 3],
}

/// Circle `center` at `radius`, `height` above the centre, from one bearing
/// to another (degrees; 0 is +Z, 90 is +X).
#[derive(Deserialize, Clone, Copy, Debug)]
#[serde(deny_unknown_fields)]
pub struct Orbit {
    pub center: [f32; 3],
    pub radius: f32,
    pub height: f32,
    pub from: f32,
    pub to: f32,
}

/// Ride behind whoever is nearest `near` when the shot starts.
#[derive(Deserialize, Clone, Copy, Debug)]
#[serde(deny_unknown_fields)]
pub struct Follow {
    /// `[x, z]`.
    pub near: [f32; 2],
    /// Metres behind, above and to the right of the body.
    #[serde(default = "Follow::default_back")]
    pub back: f32,
    #[serde(default = "Follow::default_up")]
    pub up: f32,
    #[serde(default)]
    pub side: f32,
    /// Seconds the camera takes to catch up; bigger is lazier.
    #[serde(default = "Follow::default_lag")]
    pub lag: f32,
    /// Follow an animal rather than a person.
    #[serde(default)]
    pub animal: bool,
}

impl Follow {
    fn default_back() -> f32 {
        4.5
    }
    fn default_up() -> f32 {
        2.2
    }
    fn default_lag() -> f32 {
        0.45
    }
}

/// Aim at whoever is nearest `near` when the shot starts — a tripod that
/// pans. Safer than [`Follow`] around a base: the camera stays where the
/// keys put it, so it is never inside a wall.
#[derive(Deserialize, Clone, Copy, Debug)]
#[serde(deny_unknown_fields)]
pub struct Track {
    /// `[x, z]`.
    pub near: [f32; 2],
    /// Metres above the feet to aim at.
    #[serde(default = "Track::default_up")]
    pub up: f32,
    /// Seconds the aim takes to catch up; bigger is a lazier operator.
    #[serde(default = "Track::default_lag")]
    pub lag: f32,
    #[serde(default)]
    pub animal: bool,
}

impl Track {
    fn default_up() -> f32 {
        1.1
    }
    fn default_lag() -> f32 {
        0.6
    }
}

fn weather_code(name: &str) -> Option<u8> {
    use sim_core::weather::*;
    Some(match name {
        "clear" => CLEAR,
        "overcast" => OVERCAST,
        "fog" => FOG,
        "rain" => RAIN_MILD,
        "heavy" => RAIN_HEAVY,
        "storm" => STORM,
        _ => return None,
    })
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Phase {
    /// The world is loading from the first seconds of tape.
    Boot,
    /// Jump the tape to the next shot's start, less its settle.
    Seek,
    /// Holding the shot's first pose while the rings re-centre.
    Settle { frames: u32 },
    /// Rolling frame `frame` of the shot.
    Roll { frame: u32 },
    /// Every shot is rolled; waiting for the last frames to reach ffmpeg.
    Wrap { frames: u32 },
}

/// The film's whole state. Inserted by `bin/gates.rs` with the replay.
#[derive(Resource)]
pub struct Film {
    script: Script,
    replay: crate::film::Replay,
    shot: usize,
    phase: Phase,
    /// The shot's key path in absolute metres, resolved at its seek.
    path: Vec<(f32, Vec3, Vec3)>,
    /// A follow or track shot's subject, and the camera and aim it last
    /// smoothed to.
    subject: Option<u32>,
    chase: Option<(Vec3, Vec3)>,
    encoders: Vec<Arc<Mutex<Encoder>>>,
    wav: Option<Wav>,
    /// Frames the whole run has drawn, for the log.
    drawn: u64,
}

impl Film {
    pub fn new(script: Script, replay: crate::film::Replay) -> Self {
        Self {
            script,
            replay,
            shot: 0,
            phase: Phase::Boot,
            path: Vec::new(),
            subject: None,
            chase: None,
            encoders: Vec::new(),
            wav: None,
            drawn: 0,
        }
    }

    fn cur(&self) -> &Shot {
        &self.script.shots[self.shot]
    }

    fn frames(&self) -> u32 {
        (self.cur().len * self.script.fps as f64).round().max(1.0) as u32
    }

    fn frame_ms(&self) -> f64 {
        1000.0 / self.script.fps as f64
    }

    fn stem(&self) -> PathBuf {
        let s = self.cur();
        self.script.out.join(format!("{:02}-{}", self.shot, s.name))
    }
}

/// The game's mix, rendered a frame at a time instead of by a sound card.
/// Non-send for the reason `audio_out::Native` is: `rtrb`'s ends are not
/// `Sync`.
pub struct Mix {
    pub feed: super::audio_out::Feed,
    buf: Vec<f32>,
}

impl Mix {
    pub fn new(feed: super::audio_out::Feed) -> Self {
        Self {
            feed,
            buf: Vec::new(),
        }
    }
}

/// Set this frame's world speed before `Time` advances: the shot's while it
/// rolls, real time otherwise.
pub fn steer(film: Res<Film>, mut time: ResMut<Time<Virtual>>) {
    let speed = match film.phase {
        Phase::Roll { .. } => film.cur().speed,
        _ => 1.0,
    };
    if time.relative_speed_f64() != speed {
        time.set_relative_speed_f64(speed);
    }
}

/// Hand the session everything the tape holds up to the world clock — and on
/// a seek, run the tape forward to the next shot without drawing it.
pub fn feed(
    mut film: ResMut<Film>,
    mut net: NonSendMut<Net>,
    world: Res<WorldId>,
    time: Res<Time>,
    mut day: ResMut<super::rig::DayPin>,
    mut sky: ResMut<super::weather::WeatherPin>,
) {
    if film.phase == Phase::Seek {
        let settle_ms = film.script.settle as f64 * film.frame_ms();
        let target = film.cur().at * 1000.0 - settle_ms;
        let from = film.replay.clock_ms;
        let step = client_core::clock::TICK_MS;
        while film.replay.clock_ms + step <= target {
            while !film.replay.caught_up() {
                if film.replay.feed() == 0 {
                    net.session.pump(0.0);
                }
            }
            net.session.pump(step);
            film.replay.clock_ms += step;
        }
        let shot = film.cur().clone();
        if target + 1.0 < from {
            warn!(
                "film: `{}` asks for {:.1} s but the tape is already at {:.1} s",
                shot.name,
                shot.at,
                (from + settle_ms) / 1000.0
            );
        }
        if film.replay.end_ms() < (shot.at + shot.len * shot.speed) * 1000.0 {
            warn!(
                "film: `{}` runs past the end of the tape ({:.1} s)",
                shot.name,
                film.replay.end_ms() / 1000.0
            );
        }
        info!(
            "film: {} `{}` — tape {:.1} s, {:.1} s at {}x",
            film.shot, shot.name, shot.at, shot.len, shot.speed
        );
        film.path = shot
            .keys
            .iter()
            .map(|k| (k.t, above(&world, k.pos), above(&world, k.look)))
            .collect();
        film.subject = match (shot.follow, shot.track) {
            (Some(f), _) => nearest(&net, f.near, f.animal),
            (_, Some(t)) => nearest(&net, t.near, t.animal),
            _ => None,
        };
        film.chase = None;
        *sky = super::weather::WeatherPin(shot.weather.as_deref().and_then(weather_code));
        if shot.hour.is_none() {
            *day = super::rig::DayPin(None);
        }
        film.phase = Phase::Settle { frames: 0 };
    }
    // Everything at or before the clock goes in ahead of this frame's pump
    // (`input::place_eye`), which then advances the core by this frame's
    // world time — the order the recorder stamped them in. A time-lapse
    // frame carries more snapshots than the ring holds (8), so a full ring
    // is drained into the core without moving its clock, as a live client
    // does with a burst.
    while !film.replay.caught_up() {
        if film.replay.feed() == 0 {
            net.session.pump(0.0);
        }
    }
    film.replay.clock_ms += time.delta_secs_f64() * 1000.0;
}

/// A point `[x, above ground, z]` in absolute metres.
fn above(world: &WorldId, p: [f32; 3]) -> Vec3 {
    Vec3::new(p[0], ground(world, p[0], p[2]) + p[1], p[2])
}

fn ground(world: &WorldId, x: f32, z: f32) -> f32 {
    terrain::ground(world.seed, &world.haven, x, z).max(terrain::SEA_LEVEL)
}

/// The body nearest `near`, within AOI.
fn nearest(net: &Net, near: [f32; 2], animal: bool) -> Option<u32> {
    let core = &net.session.core;
    let at = core.render_tick();
    let mut rs = client_core::interp::RemoteState::default();
    let mut best: Option<(u32, f32)> = None;
    for id in core.interp.ids() {
        if id == core.player_id || sim_core::mob::slot_of_id(id).is_some() != animal {
            continue;
        }
        if !core.interp.sample(id, at, &mut rs) || rs.dead {
            continue;
        }
        let d = (rs.x - near[0]).powi(2) + (rs.z - near[1]).powi(2);
        if best.is_none_or(|b| d < b.1) {
            best = Some((id, d));
        }
    }
    match best {
        Some((id, d)) => info!("film: subject {id}, {:.1} m from {near:?}", d.sqrt()),
        None => warn!("film: nobody near {near:?} to film"),
    }
    best.map(|b| b.0)
}

/// Point the eye for this frame. After `place_eye`, which put it where the
/// recorder stood, and before everything that streams around it.
pub fn aim(
    mut film: ResMut<Film>,
    mut eye: ResMut<Eye>,
    mut settings: ResMut<super::Settings>,
    mut day: ResMut<super::rig::DayPin>,
    net: NonSend<Net>,
    world: Res<WorldId>,
) {
    let (frame, settling) = match film.phase {
        Phase::Settle { .. } => (0, true),
        Phase::Roll { frame } => (frame, false),
        _ => return,
    };
    let fps = film.script.fps as f32;
    let shot = film.cur().clone();
    let t = frame as f32 / fps;
    let len = shot.len as f32;
    let mut u = (t / len).clamp(0.0, 1.0);
    if shot.ease {
        u = u * u * (3.0 - 2.0 * u);
    }
    let fov = shot.fov.map_or(film.script.fov, |f| f.at(u));
    if (settings.fov_deg - fov).abs() > 1e-4 {
        settings.fov_deg = fov;
    }
    if let Some(h) = shot.hour {
        *day = super::rig::DayPin::capture_at(h.at(u));
    }

    let (pos, look) = if let Some(o) = shot.orbit {
        let c = above(&world, o.center);
        let th = (o.from + (o.to - o.from) * u).to_radians();
        let p = c + Vec3::new(o.radius * th.sin(), o.height, o.radius * th.cos());
        tracked(&mut film, &net, &shot, settling, fps, p, c)
    } else if let Some(f) = shot.follow {
        match (chase(&net, film.subject, f), film.chase) {
            (Some((p, l)), last) => {
                let (p, l) = match (settling, last) {
                    (false, Some((cp, cl))) => {
                        let k = 1.0 - (-1.0 / (fps * f.lag.max(0.01))).exp();
                        (cp + (p - cp) * k, cl + (l - cl) * k)
                    }
                    _ => (p, l),
                };
                film.chase = Some((p, l));
                (p, l)
            }
            // The subject has left the tape: hold where the camera was, since
            // `place_eye` has already put the eye back on the recorder.
            (None, Some(last)) => last,
            (None, None) => return,
        }
    } else {
        let (p, l) = spline(&film.path, u * len);
        tracked(&mut film, &net, &shot, settling, fps, p, l)
    };
    let pos = Vec3::new(pos.x, pos.y.max(ground(&world, pos.x, pos.z) + 0.4), pos.z);
    let d = look - pos;
    let flat = (d.x * d.x + d.z * d.z).sqrt();
    eye.pos = pos;
    eye.yaw = d.x.atan2(d.z);
    eye.pitch = d.y.atan2(flat).clamp(-1.5, 1.5);
    eye.down = 0.0;
}

/// A shot's aim with its `track` applied: the camera stays at `p`, and the
/// look eases toward the subject rather than snapping to its every step.
fn tracked(
    film: &mut Film,
    net: &Net,
    shot: &Shot,
    settling: bool,
    fps: f32,
    p: Vec3,
    l: Vec3,
) -> (Vec3, Vec3) {
    let Some((tr, body)) = shot.track.zip(subject_at(net, film.subject)) else {
        return (p, l);
    };
    let want = body + Vec3::Y * tr.up;
    let l = match (settling, film.chase) {
        (false, Some((_, cl))) => {
            let k = 1.0 - (-1.0 / (fps * tr.lag.max(0.01))).exp();
            cl + (want - cl) * k
        }
        _ => want,
    };
    film.chase = Some((p, l));
    (p, l)
}

/// A subject's feet this frame, if the tape still has them.
fn subject_at(net: &Net, subject: Option<u32>) -> Option<Vec3> {
    let core = &net.session.core;
    let mut rs = client_core::interp::RemoteState::default();
    core.interp
        .sample(subject?, core.render_tick(), &mut rs)
        .then(|| Vec3::new(rs.x, rs.y, rs.z))
}

/// Where a follow shot's camera wants to be this frame, and what it looks at.
fn chase(net: &Net, subject: Option<u32>, f: Follow) -> Option<(Vec3, Vec3)> {
    let core = &net.session.core;
    let mut rs = client_core::interp::RemoteState::default();
    if !core.interp.sample(subject?, core.render_tick(), &mut rs) {
        return None;
    }
    let body = Vec3::new(rs.x, rs.y, rs.z);
    let yaw = crate::look::yaw_of_wire(rs.yaw);
    let fwd = Vec3::new(yaw.sin(), 0.0, yaw.cos());
    let right = Vec3::new(fwd.z, 0.0, -fwd.x);
    let pos = body - fwd * f.back + right * f.side + Vec3::Y * f.up;
    let look = body + Vec3::Y * 1.3 + fwd * 3.0;
    Some((pos, look))
}

/// Catmull-Rom through the keys at shot time `t`, with the ends extrapolated
/// straight so a two-key shot is a dolly at constant speed.
fn spline(path: &[(f32, Vec3, Vec3)], t: f32) -> (Vec3, Vec3) {
    let n = path.len();
    let i = path
        .windows(2)
        .position(|w| t < w[1].0)
        .unwrap_or(n.saturating_sub(2));
    let (t1, t2) = (path[i].0, path[i + 1].0);
    let u = ((t - t1) / (t2 - t1).max(1e-6)).clamp(0.0, 1.0);
    let pick = |f: fn(&(f32, Vec3, Vec3)) -> Vec3| {
        let p1 = f(&path[i]);
        let p2 = f(&path[i + 1]);
        let p0 = if i > 0 {
            f(&path[i - 1])
        } else {
            2.0 * p1 - p2
        };
        let p3 = if i + 2 < n {
            f(&path[i + 2])
        } else {
            2.0 * p2 - p1
        };
        let (u2, u3) = (u * u, u * u * u);
        0.5 * (2.0 * p1
            + (p2 - p0) * u
            + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * u2
            + (3.0 * p1 - p0 - 3.0 * p2 + p3) * u3)
    };
    (pick(|k| k.1), pick(|k| k.2))
}

/// End the frame: render its slice of the mix, shoot it if the shot is
/// rolling, and move the film on. After the audio flush, so this frame's cues
/// are in the slice this frame's picture goes with.
#[allow(clippy::too_many_arguments)]
pub fn roll(
    mut commands: Commands,
    mut film: ResMut<Film>,
    mut mix: NonSendMut<Mix>,
    mut exit: MessageWriter<AppExit>,
    eye: Res<Eye>,
    ring: Res<Ring>,
    props: Res<PropRing>,
    clutter: Res<ClutterRing>,
    screen: Res<State<super::Screen>>,
) {
    film.drawn += 1;
    let n = (FILM_RATE / film.script.fps) as usize * 2;
    mix.buf.resize(n, 0.0);
    let Mix { feed, buf } = &mut *mix;
    feed.fill(buf, 2);
    let ready = eye.placed
        && *screen.get() == super::Screen::InWorld
        && ring.is_full()
        && props.is_full()
        && clutter.is_full();

    match film.phase {
        Phase::Boot => {
            if ready {
                info!("film: world built at frame {}", film.drawn);
                film.phase = Phase::Seek;
            }
        }
        // `feed` always leaves a seek before this runs.
        Phase::Seek => {}
        Phase::Settle { frames } => {
            let frames = frames + 1;
            let want = film.script.settle;
            if frames >= SETTLE_MAX_FRAMES.max(want) || (frames >= want && ready) {
                if !ready {
                    warn!(
                        "film: rolling `{}` before the rings filled",
                        film.cur().name
                    );
                }
                if let Err(e) = start_shot(&mut film) {
                    error!("film: {e}");
                    exit.write(AppExit::error());
                    return;
                }
                film.phase = Phase::Roll { frame: 0 };
            } else {
                film.phase = Phase::Settle { frames };
            }
        }
        Phase::Roll { frame } => {
            if let Some(w) = &mut film.wav {
                w.push(buf);
            }
            let enc = film
                .encoders
                .last()
                .cloned()
                .expect("a rolling shot has an encoder");
            commands.spawn(Screenshot::primary_window()).observe(
                move |cap: On<ScreenshotCaptured>| {
                    let rgb = match cap.image.clone().try_into_dynamic() {
                        Ok(img) => img.to_rgb8().into_raw(),
                        Err(e) => {
                            error!("film: frame {frame} did not convert: {e:?}");
                            return;
                        }
                    };
                    if let Ok(mut enc) = enc.lock() {
                        enc.put(frame, rgb);
                    }
                },
            );
            let total = film.frames();
            if frame + 1 < total {
                film.phase = Phase::Roll { frame: frame + 1 };
            } else {
                if let Some(enc) = film.encoders.last() {
                    if let Ok(mut enc) = enc.lock() {
                        enc.total = Some(total);
                        enc.flush();
                    }
                }
                if let Some(w) = film.wav.take() {
                    if let Err(e) = w.finish() {
                        error!("film: the mix did not close: {e}");
                    }
                }
                info!("film: `{}` rolled, {total} frames", film.cur().name);
                if film.shot + 1 < film.script.shots.len() {
                    film.shot += 1;
                    film.phase = Phase::Seek;
                } else {
                    film.phase = Phase::Wrap { frames: 0 };
                }
            }
        }
        Phase::Wrap { frames } => {
            let done = film
                .encoders
                .iter()
                .all(|e| e.lock().map(|mut e| e.done()).unwrap_or(true));
            if done {
                info!(
                    "film: {} shot(s) in {}",
                    film.encoders.len(),
                    film.script.out.display()
                );
                exit.write(AppExit::Success);
            } else if frames > 600 {
                error!("film: frames never reached ffmpeg");
                exit.write(AppExit::error());
            } else {
                film.phase = Phase::Wrap { frames: frames + 1 };
            }
        }
    }
}

fn start_shot(film: &mut Film) -> Result<(), String> {
    std::fs::create_dir_all(&film.script.out)
        .map_err(|e| format!("{}: {e}", film.script.out.display()))?;
    let stem = film.stem();
    let enc = Encoder::open(
        &stem.with_extension("mp4"),
        film.script.width,
        film.script.height,
        film.script.fps,
    )?;
    film.encoders.push(Arc::new(Mutex::new(enc)));
    film.wav = Some(Wav::create(&stem.with_extension("wav"))?);
    Ok(())
}

/// One shot's ffmpeg, fed raw RGB in frame order. Readbacks land a few frames
/// after they are asked for and not always in order, so frames wait in
/// `pending` until their turn.
struct Encoder {
    child: Child,
    stdin: Option<ChildStdin>,
    next: u32,
    pending: BTreeMap<u32, Vec<u8>>,
    total: Option<u32>,
    size: (u32, u32),
}

impl Encoder {
    fn open(path: &Path, w: u32, h: u32, fps: u32) -> Result<Self, String> {
        let mut child = Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-y"])
            .args(["-f", "rawvideo", "-pix_fmt", "rgb24"])
            .args(["-s", &format!("{w}x{h}"), "-r", &fps.to_string(), "-i", "-"])
            .args(["-c:v", "libx264", "-preset", "slow", "-crf", "12"])
            .args(["-pix_fmt", "yuv420p", "-movflags", "+faststart"])
            .arg(path)
            .stdin(Stdio::piped())
            .spawn()
            .map_err(|e| format!("cannot start ffmpeg: {e}"))?;
        let stdin = child.stdin.take();
        Ok(Self {
            child,
            stdin,
            next: 0,
            pending: BTreeMap::new(),
            total: None,
            size: (w, h),
        })
    }

    fn put(&mut self, frame: u32, rgb: Vec<u8>) {
        let want = (self.size.0 * self.size.1 * 3) as usize;
        if rgb.len() != want {
            error!(
                "film: frame {frame} is {} bytes, not {want} — is the window {}x{}?",
                rgb.len(),
                self.size.0,
                self.size.1
            );
            return;
        }
        self.pending.insert(frame, rgb);
        self.flush();
    }

    fn flush(&mut self) {
        while let Some(rgb) = self.pending.remove(&self.next) {
            if let Some(stdin) = &mut self.stdin {
                if let Err(e) = stdin.write_all(&rgb) {
                    error!("film: ffmpeg refused frame {}: {e}", self.next);
                    self.stdin = None;
                }
            }
            self.next += 1;
        }
        if self.total.is_some_and(|t| self.next >= t) {
            // Closing stdin is what tells ffmpeg the shot is over.
            self.stdin = None;
        }
    }

    /// Every frame written and ffmpeg gone.
    fn done(&mut self) -> bool {
        self.stdin.is_none() && matches!(self.child.try_wait(), Ok(Some(_)) | Err(_))
    }
}

/// A 16-bit stereo WAV, sizes patched in at the end.
struct Wav {
    file: BufWriter<std::fs::File>,
    samples: u32,
}

impl Wav {
    fn create(path: &Path) -> Result<Self, String> {
        let f = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut w = Self {
            file: BufWriter::new(f),
            samples: 0,
        };
        w.header().map_err(|e| e.to_string())?;
        Ok(w)
    }

    fn header(&mut self) -> std::io::Result<()> {
        let data = self.samples * 2;
        let f = &mut self.file;
        f.write_all(b"RIFF")?;
        f.write_all(&(36 + data).to_le_bytes())?;
        f.write_all(b"WAVEfmt ")?;
        f.write_all(&16u32.to_le_bytes())?;
        f.write_all(&1u16.to_le_bytes())?; // PCM
        f.write_all(&2u16.to_le_bytes())?; // stereo
        f.write_all(&FILM_RATE.to_le_bytes())?;
        f.write_all(&(FILM_RATE * 4).to_le_bytes())?;
        f.write_all(&4u16.to_le_bytes())?;
        f.write_all(&16u16.to_le_bytes())?;
        f.write_all(b"data")?;
        f.write_all(&data.to_le_bytes())
    }

    fn push(&mut self, interleaved: &[f32]) {
        for s in interleaved {
            let v = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
            if self.file.write_all(&v.to_le_bytes()).is_err() {
                return;
            }
            self.samples += 1;
        }
    }

    fn finish(mut self) -> std::io::Result<()> {
        self.file.seek(SeekFrom::Start(0))?;
        self.header()?;
        self.file.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(t: f32, x: f32) -> (f32, Vec3, Vec3) {
        (t, Vec3::new(x, 10.0, 0.0), Vec3::new(x, 0.0, 50.0))
    }

    #[test]
    fn a_two_key_path_is_a_dolly_at_constant_speed() {
        let path = [key(0.0, 0.0), key(4.0, 40.0)];
        for (t, x) in [(0.0, 0.0), (1.0, 10.0), (2.0, 20.0), (4.0, 40.0)] {
            let (p, _) = spline(&path, t);
            assert!((p.x - x).abs() < 1e-3, "t {t}: {} not {x}", p.x);
        }
    }

    #[test]
    fn a_path_passes_through_every_key() {
        let path = [key(0.0, 0.0), key(2.0, 5.0), key(3.0, 30.0), key(6.0, 31.0)];
        for k in &path {
            let (p, l) = spline(&path, k.0);
            assert!((p - k.1).length() < 1e-3 && (l - k.2).length() < 1e-3);
        }
    }

    #[test]
    fn a_script_names_exactly_one_path_per_shot() {
        let ok = r#"{"recording":"a.rec","out":"o","shots":[
            {"name":"a","at":1,"len":2,"orbit":{"center":[0,0,0],"radius":5,"height":2,"from":0,"to":90}}]}"#;
        let s: Script = serde_json::from_str(ok).unwrap();
        assert!(s.shots[0].check().is_ok());
        let none = r#"{"name":"b","at":1,"len":2}"#;
        let shot: Shot = serde_json::from_str(none).unwrap();
        assert!(shot.check().is_err());
    }
}
