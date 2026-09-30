//! The hands: the one place an agent's view turns. A skill says where to
//! look ([`Intent`]); the hands get there the way a person's do — a
//! reaction time before a new target is even noticed, a flick capped at a
//! wrist's speed that lands a little past the mark, a pursuit that trails a
//! moving target and leads it only as well as the eye judged its speed, and
//! a wobble that settles while the aim stays on one thing. Primary is held
//! only while the hand believes it is on target.
//!
//! Everything here is counted in frames: the sim spends one input frame per
//! tick, so a call to [`Hands::drive`] is a tick of the hand, with no clock.
//! The wobble comes from a [`Pcg32`] seeded by the island, the body and the
//! preset, so a run replays exactly.

use super::intent::{Intent, Look, LEVEL_PITCH};
use super::tracks::Tracks;
use sim_core::collide::{Part, CAPSULE_HEIGHT_M, HEAD_BAND_M, LIMB_BAND_M};
use sim_core::input::{InputFrame, BTN_PRIMARY};
use sim_core::rng::{splitmix64, Pcg32};
use sim_core::yaw_dir;

/// The sim reads `yaw >> 8`: 256 bearings a turn.
pub const YAW_STEP_DEG: f32 = 360.0 / 256.0;
/// One wire pitch unit: 255 of them span straight down to straight up.
pub const PITCH_UNIT_DEG: f32 = 180.0 / 255.0;
/// A target further than this from the aim is reached with a flick.
pub const FLICK_MIN_DEG: f32 = 10.0;
/// A look that jumps further than this in one frame is a new target, and
/// takes a reaction before the hand answers it.
pub const RETARGET_DEG: f32 = 15.0;
/// Frames on target in a row before the aim counts as settled.
pub const SETTLE_FRAMES: u32 = 3;
/// Frames the eye averages a target's motion over.
const VEL_FRAMES: u32 = 4;
/// Frames of look history kept: the longest lag plus the motion window.
const HIST: usize = 16;
/// How much of the wobble carries from one frame to the next.
const WOBBLE_CARRY: f32 = 0.8;
/// `sqrt(1 - WOBBLE_CARRY²)`: keeps the wobble's spread at sigma.
const WOBBLE_FRESH: f32 = 0.6;
/// Keeps the reaction, lag and history windows inside the ring.
const MAX_LAG: u32 = HIST as u32 - VEL_FRAMES - 1;

/// How good the hands are. The numbers are a starting point; the arena
/// tunes them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Preset {
    Novice,
    Average,
    #[default]
    Good,
    Pro,
}

impl Preset {
    pub const ALL: [Preset; 4] = [Preset::Novice, Preset::Average, Preset::Good, Preset::Pro];

    pub fn name(self) -> &'static str {
        match self {
            Preset::Novice => "novice",
            Preset::Average => "average",
            Preset::Good => "good",
            Preset::Pro => "pro",
        }
    }

    pub fn parse(name: &str) -> Option<Preset> {
        Self::ALL.into_iter().find(|p| p.name() == name)
    }

    pub const fn skill(self) -> Skill {
        match self {
            Preset::Novice => Skill::NOVICE,
            Preset::Average => Skill::AVERAGE,
            Preset::Good => Skill::GOOD,
            Preset::Pro => Skill::PRO,
        }
    }
}

/// One pair of hands, as numbers. Frames are ticks (30 Hz).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Skill {
    pub preset: Preset,
    /// Frames from a new target to the first move toward it...
    pub react_frames: u32,
    /// ...and up to this many more, drawn per target.
    pub react_jitter: u32,
    /// The furthest the view turns in one frame, in yaw grid steps: a
    /// flick at full speed. Pitch is held to the same angle.
    pub turn_steps: u16,
    /// A flick lands past its mark by up to this share of its length.
    pub overshoot: f32,
    /// Share of the remaining error a pursuit closes each frame.
    pub gain: f32,
    /// Frames the eye's picture of a target trails the target.
    pub lag_frames: u32,
    /// Share of that lag the hand leads a moving target by...
    pub lead: f32,
    /// ...give or take this share of the lead, drawn per target.
    pub lead_error: f32,
    /// Wobble (one sigma, degrees) when an aim starts...
    pub wobble0_deg: f32,
    /// ...and where it settles while the aim stays on the target.
    pub wobble_floor_deg: f32,
    /// Frames for the wobble to close about two thirds of that gap.
    pub settle_frames: f32,
    /// Primary is held only while the aim is believed within this, degrees.
    pub fire_cone_deg: f32,
}

impl Skill {
    pub const NOVICE: Skill = Skill {
        preset: Preset::Novice,
        react_frames: 10,
        react_jitter: 3,
        turn_steps: 8,
        overshoot: 0.15,
        gain: 0.25,
        lag_frames: 6,
        lead: 0.3,
        lead_error: 0.5,
        wobble0_deg: 6.0,
        wobble_floor_deg: 2.0,
        settle_frames: 45.0,
        fire_cone_deg: 5.0,
    };
    pub const AVERAGE: Skill = Skill {
        preset: Preset::Average,
        react_frames: 8,
        react_jitter: 2,
        turn_steps: 14,
        overshoot: 0.10,
        gain: 0.35,
        lag_frames: 5,
        lead: 0.5,
        lead_error: 0.35,
        wobble0_deg: 4.5,
        wobble_floor_deg: 1.2,
        settle_frames: 30.0,
        fire_cone_deg: 3.5,
    };
    /// The default: about 210 ms to react, a 30-degree-a-frame flick
    /// (~900°/s), and a wobble settling from 3° to 0.7°.
    pub const GOOD: Skill = Skill {
        preset: Preset::Good,
        react_frames: 6,
        react_jitter: 1,
        turn_steps: 21,
        overshoot: 0.06,
        gain: 0.45,
        lag_frames: 4,
        lead: 0.7,
        lead_error: 0.25,
        wobble0_deg: 3.0,
        wobble_floor_deg: 0.7,
        settle_frames: 20.0,
        fire_cone_deg: 2.5,
    };
    pub const PRO: Skill = Skill {
        preset: Preset::Pro,
        react_frames: 5,
        react_jitter: 1,
        turn_steps: 32,
        overshoot: 0.04,
        gain: 0.6,
        lag_frames: 3,
        lead: 0.85,
        lead_error: 0.15,
        wobble0_deg: 2.0,
        wobble_floor_deg: 0.4,
        settle_frames: 14.0,
        fire_cone_deg: 2.0,
    };

    /// The most the wire yaw may change between two frames.
    pub const fn max_turn(&self) -> u16 {
        self.turn_steps << 8
    }

    fn turn_deg(&self) -> f32 {
        f32::from(self.turn_steps) * YAW_STEP_DEG
    }

    fn pitch_units(&self) -> i32 {
        (self.turn_deg() / PITCH_UNIT_DEG) as i32
    }
}

impl Default for Skill {
    fn default() -> Self {
        Preset::default().skill()
    }
}

/// What the look is on, to tell a new target from the same one moving.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Key {
    None,
    Heading,
    Point,
    Body(u32),
}

pub struct Hands {
    skill: Skill,
    rng: Pcg32,
    /// A view has been taken from the body; until then the hands hold none.
    live: bool,
    /// Where the hand means to point: yaw in [0, 360), pitch in [-90, 90],
    /// degrees. The wobble rides on top of it.
    aim: [f32; 2],
    wobble: [f32; 2],
    sigma: f32,
    /// The last view sent, on the wire grid.
    out: (u16, u8),
    key: Key,
    /// Frames driven, the hand's clock.
    frame: u32,
    react_until: u32,
    flick: Option<[f32; 2]>,
    lead_bias: f32,
    /// Where the target truly was, one entry a frame since it was taken.
    hist: [[f32; 2]; HIST],
    hist_n: u32,
    on: bool,
    on_streak: u32,
}

impl Hands {
    pub fn new(skill: Skill) -> Self {
        Self {
            skill,
            rng: Pcg32::new(0, 0),
            live: false,
            aim: [0.0; 2],
            wobble: [0.0; 2],
            sigma: 0.0,
            out: (0, 128),
            key: Key::None,
            frame: 0,
            react_until: 0,
            flick: None,
            lead_bias: 1.0,
            hist: [[0.0; 2]; HIST],
            hist_n: 0,
            on: false,
            on_streak: 0,
        }
    }

    /// A new connection: the view is taken from the next frame's base, and
    /// the wobble is seeded from the island, the body and the preset.
    pub fn reset(&mut self, seed: u64, player: u32) {
        let skill = self.skill;
        *self = Self::new(skill);
        let mix = splitmix64(seed ^ splitmix64(u64::from(player)));
        self.rng = Pcg32::new(mix, 0x4841_4e44 ^ skill.preset as u64);
    }

    pub fn skill(&self) -> &Skill {
        &self.skill
    }

    /// The view last sent: wire yaw and pitch.
    pub fn view(&self) -> (u16, u8) {
        self.out
    }

    /// The hand believes the aim is on the look's target: past the
    /// reaction and within the fire cone of where the eye puts it.
    pub fn on_target(&self) -> bool {
        self.on
    }

    /// On target for a few frames in a row: steady enough to release a
    /// drawn bow.
    pub fn settled(&self) -> bool {
        self.on_streak >= SETTLE_FRAMES
    }

    /// How far the aim, wobble included, is from where the target truly
    /// is, degrees. Zero with no target.
    pub fn aim_error_deg(&self) -> f32 {
        if self.hist_n == 0 {
            return 0.0;
        }
        let truth = self.sample(0);
        let dy = wrap(self.aim[0] + self.wobble[0] - truth[0]);
        let dp = self.aim[1] + self.wobble[1] - truth[1];
        dy.hypot(dp)
    }

    /// The wobble's current spread, one sigma, degrees.
    pub fn wobble_deg(&self) -> f32 {
        self.sigma
    }

    /// One frame: turn toward the intent's look as far as these hands can
    /// this frame, and lay the intent's movement and buttons over `base`
    /// (which carries the sequence number, and the view to start from
    /// before the hands hold one). `eye` is where this body looks from.
    pub fn drive(
        &mut self,
        intent: &Intent,
        tracks: &Tracks,
        eye: [f32; 3],
        base: InputFrame,
    ) -> InputFrame {
        if !self.live {
            self.live = true;
            self.out = (base.yaw & 0xff00, base.pitch);
            self.aim = [yaw_deg(self.out.0), pitch_deg(self.out.1)];
        }
        self.frame = self.frame.wrapping_add(1);
        let (key, truth) = match intent.look {
            Look::Keep => (Key::None, None),
            Look::Heading(yaw) => (Key::Heading, Some([yaw_deg(yaw), pitch_deg(LEVEL_PITCH)])),
            Look::Point(p) => (Key::Point, Some(direction(eye, p))),
            Look::Body { id, part } => {
                let seen = tracks
                    .aim_pose(id)
                    .map(|s| direction(eye, [s.x, s.y + part_height(part), s.z]));
                // Out of sight, the hand stays on where the body was last.
                let last = (self.key == Key::Body(id) && self.hist_n > 0).then(|| self.sample(0));
                (Key::Body(id), seen.or(last))
            }
        };
        match truth {
            Some(truth) => self.pursue(key, truth),
            None => {
                // Nothing to look at: the view rests where it is.
                self.on = false;
                self.on_streak = 0;
            }
        }
        let aimed = matches!(intent.look, Look::Point(_) | Look::Body { .. });
        if !aimed {
            self.wobble = [0.0; 2];
        }
        self.emit();
        let (move_x, move_z) = match intent.travel {
            Some(bearing) => axes(bearing, self.out.0),
            None => (intent.move_x, intent.move_z),
        };
        let mut buttons = intent.buttons;
        if aimed && !self.on {
            buttons &= !BTN_PRIMARY;
        }
        InputFrame {
            yaw: self.out.0,
            pitch: self.out.1,
            move_x,
            move_z,
            buttons,
            sel: intent.sel.unwrap_or(base.sel),
            ..base
        }
    }

    fn pursue(&mut self, key: Key, truth: [f32; 2]) {
        let jump = self.hist_n > 0 && {
            let last = self.sample(0);
            wrap(truth[0] - last[0])
                .abs()
                .max((truth[1] - last[1]).abs())
                > RETARGET_DEG
        };
        if key != self.key || self.hist_n == 0 || jump {
            self.take(key, truth);
        }
        self.hist[self.hist_n as usize % HIST] = truth;
        self.hist_n = self.hist_n.saturating_add(1);
        if self.frame < self.react_until {
            self.on = false;
            self.on_streak = 0;
            return;
        }
        let goal = self.perceived();
        let cap = self.skill.turn_deg();
        let err = [wrap(goal[0] - self.aim[0]), goal[1] - self.aim[1]];
        if self.flick.is_none() && err[0].abs().max(err[1].abs()) > FLICK_MIN_DEG {
            let past = self.skill.overshoot * (0.5 + 0.5 * self.unit());
            self.flick = Some([goal[0] + err[0] * past, goal[1] + err[1] * past]);
        }
        let step = match self.flick {
            Some(land) => {
                let e = [wrap(land[0] - self.aim[0]), land[1] - self.aim[1]];
                if e[0].abs().max(e[1].abs()) <= cap {
                    self.flick = None;
                }
                e
            }
            None => [err[0] * self.skill.gain, err[1] * self.skill.gain],
        };
        let scale = (cap / step[0].abs().max(step[1].abs()).max(f32::EPSILON)).min(1.0);
        self.aim = [
            (self.aim[0] + step[0] * scale).rem_euclid(360.0),
            (self.aim[1] + step[1] * scale).clamp(-90.0, 90.0),
        ];
        if key != Key::Heading {
            let floor = self.skill.wobble_floor_deg;
            self.sigma = floor + (self.sigma - floor) * (1.0 - 1.0 / self.skill.settle_frames);
            for axis in 0..2 {
                let fresh = self.gauss() * self.sigma * WOBBLE_FRESH;
                self.wobble[axis] = self.wobble[axis] * WOBBLE_CARRY + fresh;
            }
        }
        let left = [wrap(goal[0] - self.aim[0]), goal[1] - self.aim[1]];
        self.on =
            self.flick.is_none() && left[0].abs().max(left[1].abs()) <= self.skill.fire_cone_deg;
        self.on_streak = if self.on { self.on_streak + 1 } else { 0 };
    }

    /// A new target: noticed only after a reaction, unless it sits inside
    /// the fire cone of where the hand already points (the next tree along,
    /// a heading nudged). A body is always noticed afresh.
    fn take(&mut self, key: Key, truth: [f32; 2]) {
        let near = wrap(truth[0] - self.aim[0])
            .abs()
            .max((truth[1] - self.aim[1]).abs())
            <= self.skill.fire_cone_deg;
        let continues = near && !matches!(key, Key::Body(_));
        if !continues {
            let jitter = self.rng.next_bounded(self.skill.react_jitter + 1);
            self.react_until = self.frame + self.skill.react_frames + jitter;
            self.sigma = self.skill.wobble0_deg;
        }
        self.key = key;
        self.flick = None;
        self.hist_n = 0;
        self.lead_bias = 1.0 + self.skill.lead_error * (2.0 * self.unit() - 1.0);
        self.on_streak = 0;
    }

    /// Where the eye puts the target: as it was `lag` frames ago, carried
    /// forward along the motion the eye saw, by the share this hand leads.
    fn perceived(&self) -> [f32; 2] {
        let newest = self.hist_n.saturating_sub(1);
        let lag = self.skill.lag_frames.min(MAX_LAG).min(newest);
        let seen = self.sample(lag);
        let span = VEL_FRAMES.min(newest - lag);
        if span == 0 {
            return seen;
        }
        let then = self.sample(lag + span);
        let k = lag as f32 * self.skill.lead * self.lead_bias / span as f32;
        [
            seen[0] + wrap(seen[0] - then[0]) * k,
            (seen[1] + (seen[1] - then[1]) * k).clamp(-90.0, 90.0),
        ]
    }

    /// The target `back` frames ago (clamped to when it was taken).
    fn sample(&self, back: u32) -> [f32; 2] {
        let newest = self.hist_n.saturating_sub(1);
        let back = back.min(newest).min(HIST as u32 - 1);
        self.hist[(newest - back) as usize % HIST]
    }

    /// Put aim plus wobble on the wire grid, never turning further than
    /// the cap from the last frame sent.
    fn emit(&mut self) {
        let yaw = (self.aim[0] + self.wobble[0]).rem_euclid(360.0);
        let want = ((yaw / YAW_STEP_DEG).round() as i32).rem_euclid(256);
        let last = i32::from(self.out.0 >> 8);
        let steps = i32::from(self.skill.turn_steps);
        let turn = ((want - last + 128).rem_euclid(256) - 128).clamp(-steps, steps);
        let yaw = (((last + turn).rem_euclid(256)) as u16) << 8;
        let pitch = self.aim[1] + self.wobble[1];
        let want = ((pitch / 180.0 + 0.5) * 255.0).round().clamp(0.0, 255.0) as i32;
        let last = i32::from(self.out.1);
        let cap = self.skill.pitch_units();
        let pitch = (last + (want - last).clamp(-cap, cap)) as u8;
        self.out = (yaw, pitch);
    }

    /// Uniform in [0, 1).
    fn unit(&mut self) -> f32 {
        (self.rng.next_u32() >> 8) as f32 / (1u32 << 24) as f32
    }

    /// Roughly normal, unit spread: four uniforms, centred and scaled.
    fn gauss(&mut self) -> f32 {
        let sum: f32 = (0..4).map(|_| self.unit()).sum();
        (sum - 2.0) * 3f32.sqrt()
    }
}

/// Height above the feet of a body part's middle (`collide::Part` bits).
pub fn part_height(part: u8) -> f32 {
    match Part::from_bits(part) {
        Some(Part::Head) => CAPSULE_HEIGHT_M - HEAD_BAND_M / 2.0,
        Some(Part::Limb) => LIMB_BAND_M / 2.0,
        Some(Part::Chest) | None => (LIMB_BAND_M + CAPSULE_HEIGHT_M - HEAD_BAND_M) / 2.0,
    }
}

/// Strafe and forward that walk along a world bearing, for a view.
fn axes(bearing: u16, view: u16) -> (i8, i8) {
    let (tx, tz) = yaw_dir(bearing);
    let (fx, fz) = yaw_dir(view);
    // The sim's right hand is (fz, -fx) (`movement::step`).
    let right = tx * fz - tz * fx;
    let ahead = tx * fx + tz * fz;
    ((right * 127.0).round() as i8, (ahead * 127.0).round() as i8)
}

fn yaw_deg(yaw: u16) -> f32 {
    f32::from(yaw) * (360.0 / 65536.0)
}

fn pitch_deg(pitch: u8) -> f32 {
    (f32::from(pitch) / 255.0 - 0.5) * 180.0
}

/// The same angle, in (-180, 180].
fn wrap(deg: f32) -> f32 {
    180.0 - (180.0 - deg).rem_euclid(360.0)
}

/// Yaw and pitch, degrees, from `eye` to `to`: +Z is yaw 0 and +X a
/// quarter turn, as the sim's `yaw_dir`.
fn direction(eye: [f32; 3], to: [f32; 3]) -> [f32; 2] {
    let (dx, dy, dz) = (to[0] - eye[0], to[1] - eye[1], to[2] - eye[2]);
    [
        dx.atan2(dz).to_degrees().rem_euclid(360.0),
        dy.atan2(dx.hypot(dz)).to_degrees(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::intent::{pitch_toward, yaw_toward};

    const EYE: [f32; 3] = [100.0, 10.0, 100.0];

    fn hands(skill: Skill, seed: u64) -> Hands {
        let mut h = Hands::new(skill);
        h.reset(seed, 7);
        h
    }

    fn base() -> InputFrame {
        InputFrame {
            seq: 7,
            yaw: 0,
            pitch: LEVEL_PITCH,
            sel: 3,
            ..InputFrame::default()
        }
    }

    fn at(p: [f32; 3]) -> Intent {
        Intent {
            look: Look::Point(p),
            ..Intent::IDLE
        }
    }

    /// A point `deg` clockwise of +Z, 20 m out, at eye height.
    fn bearing(deg: f32) -> [f32; 3] {
        let r = deg.to_radians();
        [EYE[0] + 20.0 * r.sin(), EYE[1], EYE[2] + 20.0 * r.cos()]
    }

    fn turned(a: u16, b: u16) -> u16 {
        (a.wrapping_sub(b) as i16).unsigned_abs()
    }

    #[test]
    fn keep_holds_the_view_and_takes_the_rest_from_the_intent() {
        let tracks = Tracks::new();
        let mut h = hands(Skill::GOOD, 1);
        let start = InputFrame {
            yaw: 0x4000,
            pitch: 90,
            ..base()
        };
        let intent = Intent {
            buttons: sim_core::input::BTN_PRIMARY,
            sel: Some(2),
            ..Intent::IDLE
        };
        for _ in 0..5 {
            let f = h.drive(&intent, &tracks, EYE, start);
            assert_eq!((f.seq, f.yaw, f.pitch), (7, 0x4000, 90));
            assert_eq!((f.buttons, f.sel, f.move_z), (BTN_PRIMARY, 2, 0));
        }
        let f = h.drive(&Intent::walk(0x4000), &tracks, EYE, start);
        assert_eq!(
            (f.sel, f.buttons, f.move_z),
            (3, 0, 127),
            "walking keeps the slot"
        );
    }

    #[test]
    fn the_reaction_delay_is_honoured() {
        let tracks = Tracks::new();
        for seed in 0..20 {
            let skill = Skill::GOOD;
            let mut h = hands(skill, seed);
            let first = (1..=40u32)
                .find(|_| h.drive(&at(bearing(90.0)), &tracks, EYE, base()).yaw != 0)
                .expect("the hand moves");
            // Frame n is the first to move once n > react.
            assert!(
                (skill.react_frames + 1..=skill.react_frames + skill.react_jitter + 1)
                    .contains(&first),
                "moved on frame {first}"
            );
        }
    }

    #[test]
    fn the_turn_cap_holds_and_a_flick_lands_past_the_mark() {
        let tracks = Tracks::new();
        let mark = bearing(120.0);
        let want = yaw_toward(mark[0] - EYE[0], mark[2] - EYE[2]);
        for skill in Preset::ALL.map(Preset::skill) {
            let mut h = hands(skill, 3);
            let (mut last, mut frames, mut past) = (0u16, 0, false);
            while frames < 120 && !h.settled() {
                let f = h.drive(&at(mark), &tracks, EYE, base());
                assert!(
                    turned(f.yaw, last) <= skill.max_turn(),
                    "{:?}",
                    skill.preset
                );
                last = f.yaw;
                frames += 1;
                // Turning clockwise: past the mark is a yaw beyond it.
                past |= f.yaw.wrapping_sub(want) as i16 > 0;
            }
            assert!(h.settled(), "{:?} never settled", skill.preset);
            let off = f32::from(turned(last, want)) * 360.0 / 65536.0;
            assert!(
                off <= skill.fire_cone_deg + 3.0 * h.wobble_deg() + YAW_STEP_DEG,
                "{:?} settled {off} degrees off",
                skill.preset
            );
            let fastest = skill.react_frames + (120.0 / skill.turn_deg()) as u32;
            assert!(frames >= fastest, "{:?} turned in {frames}", skill.preset);
            assert!(past, "{:?} landed short", skill.preset);
        }
    }

    #[test]
    fn the_wobble_shrinks_while_tracking() {
        let tracks = Tracks::new();
        let mut h = hands(Skill::GOOD, 11);
        let (mut early, mut late) = (0.0f32, 0.0f32);
        for n in 0..240u32 {
            // A target drifting slowly across the view.
            let p = bearing(40.0 + n as f32 * 0.05);
            h.drive(&at(p), &tracks, EYE, base());
            match n {
                14..=34 => early += h.aim_error_deg().powi(2),
                180..=200 => late += h.aim_error_deg().powi(2),
                _ => {}
            }
        }
        let (early, late) = ((early / 21.0).sqrt(), (late / 21.0).sqrt());
        assert!(early > 1.5 * late, "early {early} late {late}");
        assert!(late < 1.5, "settled wobble {late}");
        assert!((h.wobble_deg() - Skill::GOOD.wobble_floor_deg).abs() < 0.1);
    }

    #[test]
    fn primary_waits_until_the_hand_is_on_target() {
        let tracks = Tracks::new();
        let mut h = hands(Skill::GOOD, 5);
        let intent = Intent {
            buttons: BTN_PRIMARY,
            ..at(bearing(120.0))
        };
        let mut fired = None;
        for n in 0..60 {
            let f = h.drive(&intent, &tracks, EYE, base());
            assert_eq!(f.buttons != 0, h.on_target());
            if f.buttons != 0 {
                fired.get_or_insert(n);
            }
        }
        let fired = fired.expect("fires once on target");
        assert!(fired > Skill::GOOD.react_frames as usize);
    }

    #[test]
    fn the_same_seed_replays_the_same_hands() {
        let tracks = Tracks::new();
        let script = |seed| {
            let mut h = hands(Skill::AVERAGE, seed);
            (0..200u32)
                .map(|n| {
                    let look = match n / 50 {
                        0 => at(bearing(70.0)),
                        1 => at(bearing(70.0 - n as f32)),
                        2 => Intent::walk(0x9000),
                        _ => at([EYE[0], EYE[1] - 5.0, EYE[2] + 3.0]),
                    };
                    let f = h.drive(&look, &tracks, EYE, base());
                    (f.yaw, f.pitch)
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(script(9), script(9));
        assert_ne!(script(9), script(10));
    }

    #[test]
    fn a_point_is_faced_on_the_wire_grid_and_a_heading_level() {
        let tracks = Tracks::new();
        let mut h = hands(Skill::PRO, 2);
        let up = [EYE[0] + 6.0, EYE[1] + 4.0, EYE[2] + 8.0];
        let mut f = base();
        for _ in 0..90 {
            f = h.drive(&at(up), &tracks, EYE, base());
        }
        assert_eq!(f.yaw & 0xff, 0, "on the sim's grid");
        let want = (yaw_toward(6.0, 8.0), pitch_toward(4.0, 10.0));
        assert!(turned(f.yaw, want.0) <= 2 << 8);
        assert!(f.pitch.abs_diff(want.1) <= 2);
        for _ in 0..90 {
            f = h.drive(&Intent::walk(0x8000), &tracks, EYE, base());
        }
        assert_eq!(
            (f.yaw, f.pitch),
            (0x8000, LEVEL_PITCH),
            "no wobble on a walk"
        );
    }

    #[test]
    fn travel_walks_a_bearing_whatever_the_view() {
        let tracks = Tracks::new();
        let mut h = hands(Skill::GOOD, 4);
        let away = 0x8000u16;
        let intent = Intent {
            look: Look::Heading(0),
            travel: Some(away),
            ..Intent::IDLE
        };
        let f = h.drive(&intent, &tracks, EYE, base());
        assert_eq!((f.move_x, f.move_z), (0, -127), "facing back, walking away");
        let mut h = hands(Skill::GOOD, 4);
        let f = h.drive(
            &intent,
            &tracks,
            EYE,
            InputFrame {
                yaw: 0x4000,
                ..base()
            },
        );
        let (fx, fz) = yaw_dir(f.yaw);
        let (wx, wz) = (
            fx * f32::from(f.move_z) + fz * f32::from(f.move_x),
            fz * f32::from(f.move_z) - fx * f32::from(f.move_x),
        );
        let (ax, az) = yaw_dir(away);
        assert!(wx * ax + wz * az > 120.0, "({wx}, {wz})");
    }
}
