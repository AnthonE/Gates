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
//!     time-lapse, `[[0, 1], [2, 1], [2.4, 0.2]]` ramps into slow motion, and
//!     a still stops it;
//!   · **the shot clock** — screen seconds into the shot, which the camera
//!     path is keyed on, so a time-lapse still pans at an even pace.
//!
//! The camera is a cinema camera rather than the player's: the top graphics
//! tier and past it ([`cinematic`]), a 180° shutter, a lens with a focus and
//! an aperture, an HDR grade, a hand on it if the shot wants one, and an
//! optional supersample that ffmpeg folds back down with Lanczos. A shot with
//! `still: true` stops the world on its first frame, lets TAA converge on a
//! held camera and writes one PNG — the screenshot path.
//!
//! Bevy still only draws: the replay feeds the session's lanes and the
//! ordinary `pump` drains them, so the world on screen is exactly what a live
//! client would have drawn. The one thing this file writes is the eye.

use std::collections::BTreeMap;
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};

use bevy::post_process::dof::{DepthOfField, DepthOfFieldMode};
use bevy::post_process::effect_stack::ChromaticAberration;
use bevy::post_process::motion_blur::MotionBlur;
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::render::view::{ColorGrading, ColorGradingGlobal, ColorGradingSection};
use serde::Deserialize;
use sim_core::terrain;

use super::clutter::ClutterRing;
use super::props::PropRing;
use super::rig::EyeCam;
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
/// Frames a still holds a stopped world before it is taken: TAA's history is
/// an exponential average, and by here it has converged to a supersample.
pub const STILL_HOLD: u32 = 40;
/// The shutter a shot gets unless it names one: 180°, the cinema default.
pub const SHUTTER: f32 = 0.5;
/// The aperture a focused shot gets unless it names one.
pub const FSTOP: f32 = 2.8;

/// The graphics a film draws with: [`Quality::High`]'s frame with every row
/// a player's GPU is spared turned up. Lavapipe takes a second a frame either
/// way.
///
/// [`Quality::High`]: crate::config::Quality::High
pub fn cinematic(g: super::quality::Gfx, b: Budget) -> super::quality::Gfx {
    use crate::config::Ao;
    super::quality::Gfx {
        ao: match b.ao.unwrap_or(3) {
            0 => Ao::Off,
            1 => Ao::Low,
            2 => Ao::Medium,
            3 => Ao::High,
            _ => Ao::Ultra,
        },
        taa: true,
        bloom: true,
        shadows: true,
        cascades: 4,
        shadow_m: b.shadow_m.unwrap_or(300.0),
        shadow_map_px: b.shadow_px.unwrap_or(4096),
        tree_lod_swap_m: b.tree_lod_m.unwrap_or(160.0),
        ..g
    }
}

/// What a frame may cost: the rows a shot list turns down to fit a render
/// budget on a CPU rasterizer. Unset is [`cinematic`]'s top of the range.
#[derive(Deserialize, Clone, Copy, Debug, Default)]
#[serde(deny_unknown_fields)]
pub struct Budget {
    /// Ambient occlusion, 0 off to 4 ultra.
    pub ao: Option<u8>,
    /// One shadow cascade's map, texels a side, and how far the last reaches.
    pub shadow_px: Option<usize>,
    pub shadow_m: Option<f32>,
    /// Where a tree becomes a card.
    pub tree_lod_m: Option<f32>,
    /// Motion blur taps each way (4 unless set).
    pub blur_samples: Option<u32>,
    /// Gaussian defocus instead of bokeh: cheaper, softer.
    pub gaussian: Option<bool>,
}

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
fn one() -> u32 {
    1
}
fn real_time() -> Span {
    Span::At(1.0)
}
fn default_settle() -> u32 {
    SETTLE_FRAMES
}
fn default_hold() -> u32 {
    STILL_HOLD
}

/// The shot list, as `ci/film.sh` writes it.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Script {
    /// The tape (`bin/record.rs`).
    pub recording: PathBuf,
    /// Where the shots go: `NN-name.mp4` and `NN-name.wav`, or `NN-name.png`
    /// for a still.
    pub out: PathBuf,
    /// The size of what is written.
    #[serde(default = "default_width")]
    pub width: u32,
    #[serde(default = "default_height")]
    pub height: u32,
    /// Draw at this many times the size and fold it down with Lanczos: 2 is
    /// four samples a pixel, and four times the frame time.
    #[serde(default = "one")]
    pub supersample: u32,
    #[serde(default = "default_fps")]
    pub fps: u32,
    /// Vertical field of view in degrees for shots that do not name one.
    #[serde(default = "default_fov")]
    pub fov: f32,
    /// Frames each shot holds its first pose before rolling.
    #[serde(default = "default_settle")]
    pub settle: u32,
    /// Frames a still holds the stopped world before it is taken.
    #[serde(default = "default_hold")]
    pub hold: u32,
    /// The lens and grade every shot starts from; a shot's own `look` wins
    /// field by field.
    #[serde(default)]
    pub look: Look,
    /// What a frame may cost.
    #[serde(default)]
    pub budget: Budget,
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
        if !(1..=4).contains(&s.supersample) {
            return Err(format!("supersample {} is not 1 to 4", s.supersample));
        }
        for shot in &s.shots {
            shot.check()?;
        }
        // The tape only plays forward, so the shots are taken in tape order.
        s.shots.sort_by(|a, b| a.at.total_cmp(&b.at));
        Ok(s)
    }

    /// The window: what is drawn, before any supersample is folded down.
    pub fn window(&self) -> (u32, u32) {
        (
            self.width * self.supersample,
            self.height * self.supersample,
        )
    }
}

/// One value, one eased to another across the shot, or keys at screen
/// seconds — `[[0, 1], [2.2, 1], [2.5, 0.2]]` — eased from each to the next.
#[derive(Deserialize, Clone, Debug)]
#[serde(untagged)]
pub enum Span {
    At(f32),
    Between([f32; 2]),
    Keys(Vec<[f32; 2]>),
}

impl Span {
    /// The value at shot fraction `u`, or for keys at screen second `t`.
    fn at(&self, u: f32, t: f32) -> f32 {
        match self {
            Span::At(v) => *v,
            Span::Between([a, b]) => a + (b - a) * u,
            Span::Keys(k) => keyed(k, t),
        }
    }

    fn check(&self, what: &str) -> Result<(), String> {
        if let Span::Keys(k) = self {
            if k.is_empty() || k.windows(2).any(|w| w[1][0] <= w[0][0]) {
                return Err(format!("{what}: keys need rising times"));
            }
        }
        Ok(())
    }

    fn min(&self) -> f32 {
        match self {
            Span::At(v) => *v,
            Span::Between([a, b]) => a.min(*b),
            Span::Keys(k) => k.iter().map(|k| k[1]).fold(f32::INFINITY, f32::min),
        }
    }
}

/// Keys joined by a smoothstep, so a ramp leaves one value and settles on
/// the next rather than lurching.
fn keyed(k: &[[f32; 2]], t: f32) -> f32 {
    let (Some(first), Some(last)) = (k.first(), k.last()) else {
        return 0.0;
    };
    if t <= first[0] {
        return first[1];
    }
    for w in k.windows(2) {
        if t < w[1][0] {
            let s = ((t - w[0][0]) / (w[1][0] - w[0][0])).clamp(0.0, 1.0);
            let s = s * s * (3.0 - 2.0 * s);
            return w[0][1] + (w[1][1] - w[0][1]) * s;
        }
    }
    last[1]
}

/// What the lens is focused on.
#[derive(Deserialize, Clone, Debug)]
#[serde(untagged)]
pub enum Focus {
    On(FocusOn),
    /// Metres from the lens.
    Metres(Span),
}

#[derive(Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum FocusOn {
    /// The point the camera is aimed at.
    Look,
    /// The tracked or followed body, pulled after it like a focus puller.
    Subject,
}

/// How the camera sees: lens and grade. Every field is optional so that a
/// shot's `look` overrides the script's one field at a time.
#[derive(Deserialize, Clone, Debug, Default)]
#[serde(deny_unknown_fields)]
pub struct Look {
    /// Shutter angle as a fraction of a frame: 0.5 is 180°, 0 no blur.
    pub shutter: Option<f32>,
    /// What is sharp. Unset is everything.
    pub focus: Option<Focus>,
    /// Aperture in f-stops: smaller is a shallower field.
    pub fstop: Option<f32>,
    /// Colour fringing toward the frame's edge, a fraction of its width.
    pub fringe: Option<f32>,
    /// A hand on the camera: wobble in degrees. A span, so a blast can kick.
    pub shake: Option<Span>,
    /// Dutch angle, degrees.
    pub roll: Option<Span>,
    /// Stops over (or under) the rig's own exposure.
    pub exposure: Option<Span>,
    /// White balance: + warmer, − cooler (CIE x offset, ~±0.2).
    pub temperature: Option<f32>,
    /// + magenta, − green.
    pub tint: Option<f32>,
    /// After the tone curve; 1 is neutral.
    pub saturation: Option<f32>,
    /// About mid-grey; 1 is neutral.
    pub contrast: Option<f32>,
    /// CDL offset on the shadows; 0 is neutral, − crushes.
    pub lift: Option<f32>,
    /// CDL slope on the highlights; 1 is neutral.
    pub gain: Option<f32>,
    /// CDL power on the midtones; 1 is neutral, > 1 darkens.
    pub gamma: Option<f32>,
    /// Saturation of the shadows and the highlights alone.
    pub shadow_sat: Option<f32>,
    pub highlight_sat: Option<f32>,
}

impl Look {
    /// `self` where it says something, `base` where it does not.
    fn over(&self, base: &Look) -> Look {
        Look {
            shutter: self.shutter.or(base.shutter),
            focus: self.focus.clone().or_else(|| base.focus.clone()),
            fstop: self.fstop.or(base.fstop),
            fringe: self.fringe.or(base.fringe),
            shake: self.shake.clone().or_else(|| base.shake.clone()),
            roll: self.roll.clone().or_else(|| base.roll.clone()),
            exposure: self.exposure.clone().or_else(|| base.exposure.clone()),
            temperature: self.temperature.or(base.temperature),
            tint: self.tint.or(base.tint),
            saturation: self.saturation.or(base.saturation),
            contrast: self.contrast.or(base.contrast),
            lift: self.lift.or(base.lift),
            gain: self.gain.or(base.gain),
            gamma: self.gamma.or(base.gamma),
            shadow_sat: self.shadow_sat.or(base.shadow_sat),
            highlight_sat: self.highlight_sat.or(base.highlight_sat),
        }
    }

    fn check(&self, n: &str) -> Result<(), String> {
        for (what, s) in [
            ("shake", &self.shake),
            ("roll", &self.roll),
            ("exposure", &self.exposure),
        ] {
            if let Some(s) = s {
                s.check(&format!("shot `{n}` {what}"))?;
            }
        }
        if let Some(Focus::Metres(s)) = &self.focus {
            s.check(&format!("shot `{n}` focus"))?;
        }
        if self.fstop.is_some_and(|f| f <= 0.0) {
            return Err(format!("shot `{n}`: fstop must be positive"));
        }
        Ok(())
    }

    /// The grade at this moment of the shot.
    fn grading(&self, u: f32, t: f32) -> ColorGrading {
        let section = |sat: Option<f32>| ColorGradingSection {
            saturation: sat.unwrap_or(1.0),
            contrast: self.contrast.unwrap_or(1.0),
            ..default()
        };
        let mut g = ColorGrading {
            global: ColorGradingGlobal {
                exposure: self.exposure.as_ref().map_or(0.0, |e| e.at(u, t)),
                temperature: self.temperature.unwrap_or(0.0),
                tint: self.tint.unwrap_or(0.0),
                post_saturation: self.saturation.unwrap_or(1.0),
                ..default()
            },
            shadows: section(self.shadow_sat),
            midtones: section(None),
            highlights: section(self.highlight_sat),
        };
        g.shadows.lift = self.lift.unwrap_or(0.0);
        g.highlights.gain = self.gain.unwrap_or(1.0);
        g.midtones.gamma = self.gamma.unwrap_or(1.0);
        g
    }
}

#[derive(Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Shot {
    pub name: String,
    /// Seconds into the tape at the shot's first frame.
    pub at: f64,
    /// Seconds of screen time. A still has none.
    #[serde(default)]
    pub len: f64,
    /// Tape seconds per screen second: 1 is real time, 8 a time-lapse, and
    /// keys ramp it.
    #[serde(default = "real_time")]
    pub speed: Span,
    /// Stop the world at `at` and write one PNG from the first pose.
    #[serde(default)]
    pub still: bool,
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
    /// A camera path: at least two keys (one, for a still).
    #[serde(default)]
    pub keys: Vec<Key>,
    #[serde(default)]
    pub orbit: Option<Orbit>,
    #[serde(default)]
    pub follow: Option<Follow>,
    /// Keep a body in frame: the keys move the camera, this aims it.
    #[serde(default)]
    pub track: Option<Track>,
    /// This shot's lens and grade, over the script's.
    #[serde(default)]
    pub look: Look,
}

impl Shot {
    fn check(&self) -> Result<(), String> {
        let n = &self.name;
        let keyed = self.keys.len() >= 2 || (self.still && self.keys.len() == 1);
        let paths = usize::from(keyed)
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
        if self.at < 0.0 || (!self.still && (self.len <= 0.0 || self.speed.min() <= 0.0)) {
            return Err(format!("shot `{n}`: at, len and speed must be positive"));
        }
        for (what, s) in [
            ("speed", Some(&self.speed)),
            ("hour", self.hour.as_ref()),
            ("fov", self.fov.as_ref()),
        ] {
            if let Some(s) = s {
                s.check(&format!("shot `{n}` {what}"))?;
            }
        }
        if let Some(o) = &self.orbit {
            o.radius.check(&format!("shot `{n}` orbit radius"))?;
            o.height.check(&format!("shot `{n}` orbit height"))?;
        }
        if let Some(w) = &self.weather {
            weather_code(w).ok_or_else(|| format!("shot `{n}`: unknown weather `{w}`"))?;
        }
        self.look.check(n)
    }

    /// Tape seconds the shot spans: its speed summed frame by frame.
    fn tape_s(&self, fps: u32) -> f64 {
        if self.still {
            return 0.0;
        }
        let frames = (self.len * fps as f64).round().max(1.0) as u32;
        (0..frames)
            .map(|f| {
                let t = f as f32 / fps as f32;
                self.speed.at(t / self.len as f32, t) as f64
            })
            .sum::<f64>()
            / fps as f64
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
/// to another (degrees; 0 is +Z, 90 is +X). Radius and height may ease, which
/// is a push-in or a crane.
#[derive(Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub struct Orbit {
    pub center: [f32; 3],
    pub radius: Span,
    pub height: Span,
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

/// The lens this frame, as `aim` worked it out for `lens` to put on the
/// camera.
#[derive(Default)]
struct Lens {
    shutter: f32,
    /// Metres to the plane of focus, and the aperture; none is all sharp.
    focus: Option<(f32, f32)>,
    fringe: f32,
    grade: ColorGrading,
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
    /// The focus puller's distance last frame, for a `subject` focus.
    pulled: Option<f32>,
    lens: Lens,
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
            pulled: None,
            lens: Lens::default(),
            encoders: Vec::new(),
            wav: None,
            drawn: 0,
        }
    }

    fn cur(&self) -> &Shot {
        &self.script.shots[self.shot]
    }

    fn frames(&self) -> u32 {
        if self.cur().still {
            return self.script.hold.max(1);
        }
        (self.cur().len * self.script.fps as f64).round().max(1.0) as u32
    }

    pub fn budget(&self) -> Budget {
        self.script.budget
    }

    fn frame_ms(&self) -> f64 {
        1000.0 / self.script.fps as f64
    }

    fn stem(&self) -> PathBuf {
        let s = self.cur();
        self.script.out.join(format!("{:02}-{}", self.shot, s.name))
    }

    /// The shot's look over the script's.
    fn look(&self) -> Look {
        self.cur().look.over(&self.script.look)
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
/// rolls (none for a still), real time otherwise.
pub fn steer(film: Res<Film>, mut time: ResMut<Time<Virtual>>) {
    let speed = match film.phase {
        Phase::Roll { .. } if film.cur().still => 0.0,
        Phase::Roll { frame } => {
            let shot = film.cur();
            let t = frame as f32 / film.script.fps as f32;
            shot.speed.at(t / shot.len as f32, t) as f64
        }
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
        let span_s = shot.tape_s(film.script.fps);
        if film.replay.end_ms() < (shot.at + span_s) * 1000.0 {
            warn!(
                "film: `{}` runs past the end of the tape ({:.1} s)",
                shot.name,
                film.replay.end_ms() / 1000.0
            );
        }
        if shot.still {
            info!(
                "film: {} `{}` — still at tape {:.1} s",
                film.shot, shot.name, shot.at
            );
        } else {
            info!(
                "film: {} `{}` — tape {:.1} s, {:.1} s on screen, {:.1} s of tape",
                film.shot, shot.name, shot.at, shot.len, span_s
            );
        }
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
        film.pulled = None;
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

/// Point the eye for this frame, and work out the lens. After `place_eye`,
/// which put it where the recorder stood, and before everything that streams
/// around it.
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
    let look = film.look();
    // A still is its first pose, held: nothing on it may move with the frame.
    let t = if shot.still { 0.0 } else { frame as f32 / fps };
    let len = (shot.len as f32).max(1e-3);
    let mut u = (t / len).clamp(0.0, 1.0);
    if shot.ease {
        u = u * u * (3.0 - 2.0 * u);
    }
    let fov = shot.fov.as_ref().map_or(film.script.fov, |f| f.at(u, t));
    if (settings.fov_deg - fov).abs() > 1e-4 {
        settings.fov_deg = fov;
    }
    if let Some(h) = &shot.hour {
        *day = super::rig::DayPin::capture_at(h.at(u, t));
    }

    let (pos, aim_at) = if let Some(o) = &shot.orbit {
        let c = above(&world, o.center);
        let th = (o.from + (o.to - o.from) * u).to_radians();
        let r = o.radius.at(u, t);
        let p = c + Vec3::new(r * th.sin(), o.height.at(u, t), r * th.cos());
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
        let (p, l) = spline(&film.path, if shot.still { 0.0 } else { u * len });
        tracked(&mut film, &net, &shot, settling, fps, p, l)
    };
    let pos = Vec3::new(pos.x, pos.y.max(ground(&world, pos.x, pos.z) + 0.4), pos.z);
    let d = aim_at - pos;
    let flat = (d.x * d.x + d.z * d.z).sqrt();
    // The hand on the camera: three slow incommensurate sines an axis, so it
    // drifts and never repeats inside a shot, seeded by the shot's number so
    // two cuts in a row do not wobble alike.
    let amp = look.shake.as_ref().map_or(0.0, |s| s.at(u, t)).to_radians();
    let seed = film.shot as f32 * 7.31;
    let (wy, wp, wr) = if shot.still || amp == 0.0 {
        (0.0, 0.0, 0.0)
    } else {
        (
            amp * wobble(t, seed),
            amp * 0.8 * wobble(t, seed + 11.7),
            amp * 0.5 * wobble(t, seed + 23.1),
        )
    };
    eye.pos = pos;
    eye.yaw = d.x.atan2(d.z) + wy;
    eye.pitch = (d.y.atan2(flat) + wp).clamp(-1.5, 1.5);
    eye.roll = look.roll.as_ref().map_or(0.0, |r| r.at(u, t)).to_radians() + wr;
    eye.down = 0.0;

    // The lens. A `subject` focus is pulled after the body at a focus
    // puller's pace; a `look` focus is wherever the camera is aimed.
    let fstop = look.fstop.unwrap_or(FSTOP);
    let focus = match &look.focus {
        None => None,
        Some(Focus::Metres(m)) => Some(m.at(u, t)),
        Some(Focus::On(FocusOn::Look)) => Some(pos.distance(aim_at)),
        Some(Focus::On(FocusOn::Subject)) => {
            let want = subject_at(&net, film.subject)
                .map_or(pos.distance(aim_at), |b| pos.distance(b + Vec3::Y * 1.2));
            let d = match (settling, film.pulled) {
                (false, Some(last)) => last + (want - last) * (1.0 - (-1.0 / (fps * 0.3)).exp()),
                _ => want,
            };
            film.pulled = Some(d);
            Some(d)
        }
    };
    film.lens = Lens {
        shutter: if shot.still {
            0.0
        } else {
            look.shutter.unwrap_or(SHUTTER)
        },
        focus: focus.map(|f| (f.max(0.3), fstop)),
        fringe: look.fringe.unwrap_or(0.0),
        grade: look.grading(u, t),
    };
}

/// Smooth noise in about ±1 from three sines whose ratios never line up.
fn wobble(t: f32, seed: f32) -> f32 {
    use std::f32::consts::TAU;
    0.55 * (t * 0.37 * TAU + seed).sin()
        + 0.30 * (t * 0.91 * TAU + seed * 1.7).sin()
        + 0.15 * (t * 2.13 * TAU + seed * 2.3).sin()
}

/// Put this frame's lens on the camera: shutter, focus, fringe and grade.
/// The components come and go with the shots that want them, which costs a
/// re-specialise — inside the settle, where nothing is being written.
#[allow(clippy::type_complexity)]
pub fn lens(
    mut commands: Commands,
    film: Res<Film>,
    window: Query<&Window, With<bevy::window::PrimaryWindow>>,
    mut cam: Query<
        (
            Entity,
            &mut ColorGrading,
            Option<&mut MotionBlur>,
            Option<&mut DepthOfField>,
            Option<&mut ChromaticAberration>,
        ),
        With<EyeCam>,
    >,
) {
    if !matches!(film.phase, Phase::Settle { .. } | Phase::Roll { .. }) {
        return;
    }
    let Ok((cam, mut grading, blur, dof, fringe)) = cam.single_mut() else {
        return;
    };
    let l = &film.lens;
    *grading = l.grade.clone();
    match blur {
        Some(mut b) => {
            if b.shutter_angle != l.shutter {
                b.shutter_angle = l.shutter;
            }
        }
        None => {
            commands.entity(cam).insert(MotionBlur {
                shutter_angle: l.shutter,
                samples: film.script.budget.blur_samples.unwrap_or(4),
            });
        }
    }
    match (l.focus, dof) {
        (Some((at, fstop)), Some(mut d)) => {
            d.focal_distance = at;
            d.aperture_f_stops = fstop;
        }
        (Some((at, fstop)), None) => {
            // The blur's cap scales with the drawn frame, so a supersampled
            // shot keeps the same look as one drawn at size.
            let h = window
                .single()
                .map_or(1080.0, |w| w.physical_height() as f32);
            let gaussian = film.script.budget.gaussian.unwrap_or(false);
            commands.entity(cam).insert(DepthOfField {
                mode: if gaussian {
                    DepthOfFieldMode::Gaussian
                } else {
                    DepthOfFieldMode::Bokeh
                },
                focal_distance: at,
                aperture_f_stops: fstop,
                max_circle_of_confusion_diameter: 48.0 * h / 1080.0,
                max_depth: 3000.0,
                ..default()
            });
        }
        (None, Some(_)) => {
            commands.entity(cam).remove::<DepthOfField>();
        }
        (None, None) => {}
    }
    match (l.fringe > 0.0, fringe) {
        (true, Some(mut f)) => f.intensity = l.fringe,
        (true, None) => {
            commands.entity(cam).insert(ChromaticAberration {
                intensity: l.fringe,
                max_samples: 12,
                ..default()
            });
        }
        (false, Some(_)) => {
            commands.entity(cam).remove::<ChromaticAberration>();
        }
        (false, None) => {}
    }
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
/// straight so a two-key shot is a dolly at constant speed. One key is a
/// tripod.
fn spline(path: &[(f32, Vec3, Vec3)], t: f32) -> (Vec3, Vec3) {
    let n = path.len();
    if n == 1 {
        return (path[0].1, path[0].2);
    }
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
            let total = film.frames();
            let still = film.cur().still;
            // A still is taken once, on the last frame of its hold.
            if !still || frame + 1 == total {
                let enc = film
                    .encoders
                    .last()
                    .cloned()
                    .expect("a rolling shot has an encoder");
                let slot = if still { 0 } else { frame };
                commands.spawn(Screenshot::primary_window()).observe(
                    move |cap: On<ScreenshotCaptured>| {
                        let rgb = match cap.image.clone().try_into_dynamic() {
                            Ok(img) => img.to_rgb8().into_raw(),
                            Err(e) => {
                                error!("film: frame {slot} did not convert: {e:?}");
                                return;
                            }
                        };
                        if let Ok(mut enc) = enc.lock() {
                            enc.put(slot, rgb);
                        }
                    },
                );
            }
            if frame + 1 < total {
                film.phase = Phase::Roll { frame: frame + 1 };
            } else {
                let written = if still { 1 } else { total };
                if let Some(enc) = film.encoders.last() {
                    if let Ok(mut enc) = enc.lock() {
                        enc.total = Some(written);
                        enc.flush();
                    }
                }
                if let Some(w) = film.wav.take() {
                    if let Err(e) = w.finish() {
                        error!("film: the mix did not close: {e}");
                    }
                }
                info!("film: `{}` rolled, {written} frame(s)", film.cur().name);
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
    let s = &film.script;
    let (out, drawn) = ((s.width, s.height), s.window());
    let enc = if film.cur().still {
        Encoder::open(&stem.with_extension("png"), drawn, out, None)?
    } else {
        let enc = Encoder::open(&stem.with_extension("mp4"), drawn, out, Some(s.fps))?;
        film.wav = Some(Wav::create(&stem.with_extension("wav"))?);
        enc
    };
    film.encoders.push(Arc::new(Mutex::new(enc)));
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
    /// Frames `drawn` in size, written `out` in size: a supersample is folded
    /// down with Lanczos here. No `fps` is a still, written as a PNG.
    fn open(
        path: &Path,
        drawn: (u32, u32),
        out: (u32, u32),
        fps: Option<u32>,
    ) -> Result<Self, String> {
        let (w, h) = drawn;
        let mut cmd = Command::new("ffmpeg");
        cmd.args(["-hide_banner", "-loglevel", "error", "-y"])
            .args(["-f", "rawvideo", "-pix_fmt", "rgb24"])
            .args(["-s", &format!("{w}x{h}")]);
        if let Some(fps) = fps {
            cmd.args(["-r", &fps.to_string()]);
        }
        cmd.args(["-i", "-"]);
        if drawn != out {
            cmd.args([
                "-vf",
                &format!(
                    "scale={}:{}:flags=lanczos+accurate_rnd+full_chroma_int",
                    out.0, out.1
                ),
            ]);
        }
        match fps {
            Some(_) => cmd
                .args(["-c:v", "libx264", "-preset", "slow", "-crf", "10"])
                .args(["-pix_fmt", "yuv420p", "-movflags", "+faststart"]),
            None => cmd.args(["-frames:v", "1"]),
        };
        let mut child = cmd
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
        // A still needs no length and may stand on one key.
        let still =
            r#"{"name":"c","at":1,"still":true,"keys":[{"t":0,"pos":[0,2,0],"look":[9,1,9]}]}"#;
        let shot: Shot = serde_json::from_str(still).unwrap();
        assert!(shot.check().is_ok());
    }

    #[test]
    fn a_ramp_holds_eases_and_holds() {
        let s: Span = serde_json::from_str("[[0, 1], [2, 1], [3, 0.2]]").unwrap();
        assert_eq!(s.at(0.0, 0.0), 1.0);
        assert_eq!(s.at(0.0, 1.5), 1.0);
        let mid = s.at(0.0, 2.5);
        assert!((mid - 0.6).abs() < 1e-4, "{mid}");
        assert_eq!(s.at(0.0, 9.0), 0.2);
        // A pair is still a straight ease across the shot, not two keys.
        let pair: Span = serde_json::from_str("[10, 20]").unwrap();
        assert_eq!(pair.at(0.5, 99.0), 15.0);
    }

    #[test]
    fn a_ramp_spends_the_tape_it_says() {
        let shot: Shot = serde_json::from_str(
            r#"{"name":"r","at":0,"len":4,"speed":[[0,1],[2,1],[2.001,0.25]],
                "orbit":{"center":[0,0,0],"radius":5,"height":2,"from":0,"to":9}}"#,
        )
        .unwrap();
        let s = shot.tape_s(30);
        assert!((s - 2.5).abs() < 0.05, "{s}");
    }

    #[test]
    fn a_shot_look_wins_field_by_field() {
        let base: Look = serde_json::from_str(r#"{"fstop": 4, "saturation": 1.1}"#).unwrap();
        let shot: Look = serde_json::from_str(r#"{"fstop": 1.8, "focus": "subject"}"#).unwrap();
        let l = shot.over(&base);
        assert_eq!(l.fstop, Some(1.8));
        assert_eq!(l.saturation, Some(1.1));
        assert!(matches!(l.focus, Some(Focus::On(FocusOn::Subject))));
        let m: Look = serde_json::from_str(r#"{"focus": [3, 12]}"#).unwrap();
        assert!(matches!(m.focus, Some(Focus::Metres(Span::Between(_)))));
    }
}
