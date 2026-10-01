//! What this body's eyes and ears know of other bodies.
//!
//! The snapshot stream names every body in the server's interest set, the
//! way it does for a human client, which draws them all. A human still only
//! knows the ones they looked at. So bodies reach the agent only through
//! perception: inside a 90° cone of its own facing, within a range that
//! shrinks at night and in fog, with a line of sight held for a few ticks.
//! Once out of sight a track keeps the pose it was last seen in, and is
//! forgotten after a while. Sounds (shots, swings, impacts, blows, deaths)
//! arrive as a bearing and a distance band, never as a body.
//!
//! Poses are sampled from this body's own interpolation of the snapshot
//! ring at `newest − playout`: what a human's screen shows, and the pose
//! the server rewinds melee and hitscan to (`stats.rs` `favour_for`).
//!
//! Everything is fixed-size and allocated once, in [`Tracks::new`]; the
//! per-frame work is bounded by the budget constants below.

use client_core::core::ClientCore;
use client_core::interp::{Interp, RemoteState};
use client_core::view::ClientView;
use sim_core::limits::{
    ARROW_STEP_MM, DAY_PHASE_TICKS, DAY_PORTION, DAY_TICKS, INTERP_DELAY_TICKS, TICK_HZ,
};
use sim_core::movement::{POS_XZ_Q, POS_Y_Q, SPRINT_SPEED, TERMINAL_VELOCITY, WALK_SPEED};
use sim_core::ranged::{eye_mm, MM_PER_M};
use sim_core::rng::Pcg32;
use sim_core::terrain::{self, Haven};
use sim_core::weather::{self, Env};
use sim_core::{collide, mob, pitch_dir, yaw_dir};

/// Bodies remembered at once. A full table forgets the one out of sight
/// longest, and while every row is in sight a newcomer waits.
pub const TRACK_ROWS: usize = 32;
/// How far a player is picked out in daylight and clear air.
pub const PLAYER_SIGHT_M: f32 = 120.0;
/// Animals are smaller and drab: about half that.
pub const ANIMAL_SIGHT_M: f32 = 60.0;
/// The share of those ranges left at night for a body carrying no light.
pub const NIGHT_SIGHT: f32 = 0.3;
/// A line of sight must still be clear this many ticks after it first was
/// before a body counts as seen: a glimpse is not a sighting.
pub const SPOT_TICKS: u32 = 4;
/// A blocked sight line's answer is trusted this long before it is cast
/// again. A clear one is recast every [`SPOT_TICKS`], and a body goes out of
/// sight once its clear answer is older than this: a body that steps behind
/// a wall is lost about as fast as a human's eye loses it.
pub const LOS_CACHE_TICKS: u32 = 8;
/// Rays at most this long are cheap; longer ones have their own budget.
pub const SHORT_RAY_M: f32 = 32.0;
/// Sight rays cast per frame, by length. Together they bound a frame's
/// perception cost whatever the crowd.
pub const LONG_RAYS_PER_FRAME: u32 = 2;
pub const SHORT_RAYS_PER_FRAME: u32 = 4;
/// Snapshots fed per frame when catching up; the interpolation history is
/// no deeper than this anyway.
pub const FEED_MAX_TICKS: u32 = 16;
/// An unseen track is forgotten this long after it was last seen.
pub const FORGET_TICKS: u32 = 30 * TICK_HZ;
/// A look ray passing this close to my chest is aimed at me.
pub const AIM_MISS_M: f32 = 1.0;
/// Terrain is tried along a long ray at this spacing before the fine walk.
pub const COARSE_STEP_M: f32 = 2.0;
/// Sounds kept, newest first out.
pub const HEARD_ROWS: usize = 8;
/// Beyond these a sound is not heard.
pub const SHOT_HEAR_M: f32 = 150.0;
pub const SWING_HEAR_M: f32 = 20.0;
pub const DEATH_HEAR_M: f32 = 60.0;
/// Another player's footsteps, as the human client plays them
/// (`sound::steps`, `render/audio.rs::remote_steps`): nothing below a
/// shuffle, full gain at a sprint and never under a floor of it, and gone
/// at the step cue's radius. Heard at most this often.
pub const STEP_HEAR_M: f32 = 24.0;
pub const STEP_MIN_MPS: f32 = 0.4;
pub const STEP_FULL_MPS: f32 = SPRINT_SPEED;
pub const STEP_MIN_GAIN: f32 = 0.45;
pub const STEP_EVERY_TICKS: u32 = 8;
/// An arrow landing this close is "shot at".
pub const IMPACT_NEAR_M: f32 = 6.0;
/// A heard bearing is off by up to this much either way (wire yaw units,
/// about 10°); a heard distance by up to a quarter.
pub const HEAR_BEARING_NOISE: u32 = 1820;
/// For this long after my own shot, an arrow landing near me is taken for
/// mine: an impact names no shooter, and my miss at something close is not
/// me being shot at.
pub const OWN_SHOT_QUIET_TICKS: u32 = TICK_HZ;
/// A body coming at me at least this fast is closing.
pub const CLOSING_MPS: f32 = WALK_SPEED * 0.5;
/// Nothing runs or falls faster: a larger difference between two poses is a
/// respawn or a teleport, and says nothing about how the body moves.
const MAX_BODY_MPS: f32 = TERMINAL_VELOCITY + SPRINT_SPEED;
/// Distance bands a sound is placed in.
pub const NEAR_M: f32 = 20.0;
pub const MID_M: f32 = 60.0;

/// Eye heights above the feet the sight line is cast to: a head over a low
/// wall for a player, the back of an animal.
const PLAYER_SIGHT_Y_M: f32 = 1.4;
/// A crouched player's (v83): the bottom of its lowered head band, the
/// same 0.3 m under the top a standing one's sits.
const CROUCH_SIGHT_Y_M: f32 =
    collide::CROUCH_HEIGHT_M - (collide::CAPSULE_HEIGHT_M - PLAYER_SIGHT_Y_M);
const ANIMAL_SIGHT_Y_M: f32 = 0.5;
/// Closer than this, a body is in view across the whole width of the
/// screen, not only the cone the eyes attend to further out.
pub const ARMS_LENGTH_M: f32 = 2.0;
/// Closer than this, two bodies overlap and nothing can stand between.
const TOUCH_M: f32 = 2.0 * collide::CAPSULE_RADIUS_M;
/// The cosine of half the screen's width: the client's 75° vertical view
/// at 16:9 is about 107° across, so a body up to ~54° off the facing.
pub const SCREEN_HALF_COS: f32 = 0.59;
/// Where a look ray is tested against this body, standing and crouched
/// (v83: the crouched hit volume is 1.05 m, so its chest sits lower).
const CHEST_Y_M: f32 = 1.2;
const CROUCH_CHEST_Y_M: f32 = 0.7;

/// The eye above the feet for a stance, metres: the sim's own
/// (`ranged::eye_mm`), where a shot or a swing leaves from.
pub fn eye_m(crouched: bool) -> f32 {
    eye_mm(crouched) as f32 / MM_PER_M
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Species {
    Player,
    Pig,
    Wolf,
}

impl Species {
    pub fn of(id: u32) -> Species {
        match mob::slot_of_id(id) {
            None => Species::Player,
            Some(slot) if mob::kind_of(slot) == mob::MOB_WOLF => Species::Wolf,
            Some(_) => Species::Pig,
        }
    }

    fn sight_m(self) -> f32 {
        match self {
            Species::Player => PLAYER_SIGHT_M,
            Species::Pig | Species::Wolf => ANIMAL_SIGHT_M,
        }
    }

    fn sight_y(self, crouched: bool) -> f32 {
        match self {
            Species::Player if crouched => CROUCH_SIGHT_Y_M,
            Species::Player => PLAYER_SIGHT_Y_M,
            Species::Pig | Species::Wolf => ANIMAL_SIGHT_Y_M,
        }
    }
}

/// One body this agent has seen.
#[derive(Clone, Copy, Debug)]
pub struct Track {
    /// The wire entity id: for aiming and matching hit markers only. It
    /// never leaves the agent (no `Summary`, log line or request).
    pub id: u32,
    pub species: Species,
    pub first_seen: u32,
    pub last_seen: u32,
    /// Feet, metres, as last seen.
    pub pos: [f32; 3],
    /// Metres per second, as last seen.
    pub vel: [f32; 3],
    pub yaw: u16,
    pub pitch: u8,
    pub held: Option<u16>,
    pub lit: bool,
    pub wounded: bool,
    pub dead: bool,
    pub sleeping: bool,
    /// Crouched (wire v83): a lower eye and a shorter body to hit.
    pub crouched: bool,
    /// Damage my own blows did it, from my hit markers.
    pub dealt: u32,
    /// When I last saw it swing or shoot.
    pub last_swing: Option<u32>,
    pub last_shot: Option<u32>,
    /// Its look ray passed within [`AIM_MISS_M`] of me when last seen.
    pub aiming_at_me: bool,
    /// In sight this frame.
    pub visible: bool,
}

impl Track {
    /// Up, awake and not a corpse: something that can act.
    pub fn active(&self) -> bool {
        !self.dead && !self.sleeping
    }
}

#[derive(Clone, Copy)]
struct Row {
    track: Track,
    /// Has held a clear line for [`SPOT_TICKS`]: a track, not a glimpse.
    seen: bool,
    /// In the cone and range this frame, and the sample taken for it.
    in_view: bool,
    sample: RemoteState,
    los_at: Option<u32>,
    los_ok: bool,
    los_since: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sound {
    Shot,
    Swing,
    /// An arrow landed near me.
    Impact,
    /// A blow landed on me.
    Hurt,
    Death,
    /// Footsteps from someone out of sight.
    Step,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Band {
    Near,
    Mid,
    Far,
}

impl Band {
    fn of(m: f32) -> Band {
        if m < NEAR_M {
            Band::Near
        } else if m < MID_M {
            Band::Mid
        } else {
            Band::Far
        }
    }
}

/// One sound: which way (a world wire yaw toward it), how far, and when.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Heard {
    pub sound: Sound,
    pub bearing: u16,
    pub band: Band,
    pub tick: u32,
}

/// This body as its own snapshots show it.
#[derive(Clone, Copy, Debug, Default)]
pub struct Own {
    pub tick: u32,
    pub pos: [f32; 3],
    pub vel: [f32; 3],
    pub yaw: u16,
    pub pitch: u8,
    /// The stance the server applied (wire v83): where the eyes were.
    pub crouched: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct TrackStats {
    pub rays: u64,
    pub coarse_rejects: u64,
    pub spotted: u64,
    pub heard: u64,
}

pub struct Tracks {
    interp: Interp,
    rows: [Option<Row>; TRACK_ROWS],
    heard: [Option<Heard>; HEARD_ROWS],
    heard_next: usize,
    /// The newest snapshot tick fed into `interp`.
    fed: Option<u32>,
    me: u32,
    own: Option<Own>,
    /// When this body last fired, from its own echoed shot.
    own_shot: Option<u32>,
    /// When footsteps were last heard.
    stepped: Option<u32>,
    playout: u8,
    rng: Pcg32,
    pub stats: TrackStats,
}

impl Default for Tracks {
    fn default() -> Self {
        Self::new()
    }
}

impl Tracks {
    pub fn new() -> Self {
        Self {
            interp: Interp::new(),
            rows: [None; TRACK_ROWS],
            heard: [None; HEARD_ROWS],
            heard_next: 0,
            fed: None,
            me: 0,
            own: None,
            own_shot: None,
            stepped: None,
            playout: INTERP_DELAY_TICKS,
            rng: Pcg32::new(0, 0),
            stats: TrackStats::default(),
        }
    }

    /// A new session: nothing fed, nothing known. The sound noise is drawn
    /// from this body's own stream so two agents mishear differently.
    pub fn reset(&mut self, seed: u64, player: u32) {
        self.interp.clear();
        self.fed = None;
        self.me = player;
        self.rng = Pcg32::new(seed, u64::from(player));
        self.forget();
    }

    /// A new body on a new beach: what the old one saw is elsewhere.
    /// The old pose goes too, so no velocity is differenced across the jump.
    pub fn forget(&mut self) {
        self.rows = [None; TRACK_ROWS];
        self.heard = [None; HEARD_ROWS];
        self.own = None;
        self.own_shot = None;
        self.stepped = None;
    }

    /// The playout delay bodies are sampled behind the newest snapshot,
    /// in ticks: the session's adaptive delay where one runs, else the
    /// default. Clamped to what the interpolation history holds.
    pub fn set_playout(&mut self, ticks: u8) {
        self.playout = ticks.min(FEED_MAX_TICKS as u8 - 1);
    }

    pub fn playout(&self) -> u8 {
        self.playout
    }

    /// Take the snapshots applied since the last frame into the
    /// interpolation history, as the human client does on each datagram.
    pub fn feed(&mut self, view: &ClientView, me: u32) {
        let Some(newest) = view.newest_applied else {
            return;
        };
        self.me = me;
        let first = match self.fed {
            Some(fed) if fed == newest => return,
            Some(fed) if newest.wrapping_sub(fed) <= FEED_MAX_TICKS => fed.wrapping_add(1),
            // A first frame, a long stall or a new session: the history
            // restarts from the ticks it can still hold. Samples arrive in
            // tick order, so a view that went back in time starts over.
            fed => {
                if fed.is_some_and(|f| newest < f) {
                    self.interp.clear();
                    self.own = None;
                }
                newest.saturating_sub(FEED_MAX_TICKS - 1)
            }
        };
        self.fed = Some(newest);
        let mut t = first;
        loop {
            if let Some(snap) = view.at(t) {
                if snap.header.baseline_age == 0 {
                    self.interp.retain_present(snap.entities());
                }
                for &id in snap.removed() {
                    self.interp.remove(id);
                }
                for e in snap.entities() {
                    if e.id == me {
                        let pos = [
                            e.qx as f32 * POS_XZ_Q,
                            e.qy as f32 * POS_Y_Q,
                            e.qz as f32 * POS_XZ_Q,
                        ];
                        let vel = match self.own {
                            Some(o)
                                if t.wrapping_sub(o.tick) > 0 && t.wrapping_sub(o.tick) <= 2 =>
                            {
                                velocity(pos, o.pos, t.wrapping_sub(o.tick))
                            }
                            _ => [0.0; 3],
                        };
                        self.own = Some(Own {
                            tick: t,
                            pos,
                            vel,
                            yaw: e.yaw,
                            pitch: e.pitch,
                            crouched: e.crouched,
                        });
                    } else {
                        self.interp.push(t, e);
                    }
                }
            }
            if t == newest {
                break;
            }
            t = t.wrapping_add(1);
        }
    }

    /// Look: find the bodies in the cone and range, cast the budgeted sight
    /// lines, and bring every track in sight up to date.
    pub fn perceive(&mut self, core: &mut ClientCore, haven: &Haven, tick: u32) {
        let (Some(fed), Some(own)) = (self.fed, self.own) else {
            return;
        };
        let at = f64::from(fed) - f64::from(self.playout);
        let eye = [own.pos[0], own.pos[1] + eye_m(own.crouched), own.pos[2]];
        let (fx, fz) = yaw_dir(own.yaw);
        let (seed, _) = core.island();
        let clarity =
            weather::sense_pm(&weather::now(seed, u64::from(tick), &core.env)) as f32 / 1000.0;
        let dark = night(tick, &core.env);
        for row in self.rows.iter_mut().flatten() {
            row.in_view = false;
        }
        // The loudest footsteps from out of sight this frame.
        let mut steps: Option<(f32, [f32; 3])> = None;
        for id in self.interp.ids() {
            if id == self.me {
                continue;
            }
            let mut s = RemoteState::default();
            if !self.interp.sample(id, at, &mut s) {
                continue;
            }
            let species = Species::of(id);
            let (dx, dy, dz) = (s.x - eye[0], s.y - eye[1], s.z - eye[2]);
            let d = (dx * dx + dz * dz).sqrt();
            let mut range = species.sight_m() * clarity;
            if dark && !s.lit {
                range *= NIGHT_SIGHT;
            }
            // Range is along the line, the cone on the ground. A body at
            // arm's length is seen anywhere on the screen, not only in the
            // cone: at that range it fills the side of the view. Beside
            // me, off the screen, it is not seen at all.
            let ahead = dx * fx + dz * fz;
            let cone = if d < ARMS_LENGTH_M {
                d * SCREEN_HALF_COS
            } else {
                d * std::f32::consts::FRAC_1_SQRT_2
            };
            // A body standing in mine fills the edge of the screen
            // whichever way I face.
            if d * d + dy * dy > range * range || (ahead < cone && d >= TOUCH_M) {
                if species == Species::Player && !s.dead && !s.sleeping {
                    let mut before = RemoteState::default();
                    if self.interp.sample(id, at - 1.0, &mut before) {
                        let v = velocity([s.x, s.y, s.z], [before.x, before.y, before.z], 1);
                        let speed = (v[0] * v[0] + v[2] * v[2]).sqrt();
                        let gain = (speed / STEP_FULL_MPS).clamp(STEP_MIN_GAIN, 1.0);
                        let left = STEP_HEAR_M * gain - d;
                        if speed >= STEP_MIN_MPS
                            && left > 0.0
                            && steps.is_none_or(|(l, _)| left > l)
                        {
                            steps = Some((left, [s.x, s.y, s.z]));
                        }
                    }
                }
                continue;
            }
            let i = match self.slot_of(id) {
                Some(i) => i,
                None => {
                    let Some(i) = self.free_slot(tick) else {
                        continue;
                    };
                    self.rows[i] = Some(Row {
                        track: Track {
                            id,
                            species,
                            first_seen: tick,
                            last_seen: tick,
                            pos: [s.x, s.y, s.z],
                            vel: [0.0; 3],
                            yaw: s.yaw as u16,
                            pitch: s.pitch as u8,
                            held: s.held,
                            lit: s.lit,
                            wounded: s.wounded,
                            dead: s.dead,
                            sleeping: s.sleeping,
                            crouched: s.crouched,
                            dealt: 0,
                            last_swing: None,
                            last_shot: None,
                            aiming_at_me: false,
                            visible: false,
                        },
                        seen: false,
                        in_view: false,
                        sample: s,
                        los_at: None,
                        los_ok: false,
                        los_since: None,
                    });
                    i
                }
            };
            if let Some(row) = self.rows[i].as_mut() {
                row.in_view = true;
                row.sample = s;
            }
        }
        if let Some((_, from)) = steps {
            if self
                .stepped
                .is_none_or(|at| tick.wrapping_sub(at) >= STEP_EVERY_TICKS)
            {
                self.stepped = Some(tick);
                self.hear(Sound::Step, from, STEP_HEAR_M, true, tick);
            }
        }
        self.cast_rays(core, haven, eye, tick);
        let chest_y = if own.crouched {
            CROUCH_CHEST_Y_M
        } else {
            CHEST_Y_M
        };
        let chest = [own.pos[0], own.pos[1] + chest_y, own.pos[2]];
        for slot in self.rows.iter_mut() {
            let Some(row) = slot.as_mut() else {
                continue;
            };
            if !row.in_view {
                // Out of the cone: the next look starts a fresh sighting.
                row.los_at = None;
                row.los_since = None;
                row.los_ok = false;
            }
            let fresh = row
                .los_at
                .is_some_and(|at| tick.wrapping_sub(at) <= LOS_CACHE_TICKS);
            let visible = row.seen && row.in_view && row.los_ok && fresh;
            row.track.visible = visible;
            if visible {
                let s = row.sample;
                let t = &mut row.track;
                let mut before = RemoteState::default();
                t.vel = if self.interp.sample(t.id, at - 1.0, &mut before) {
                    velocity([s.x, s.y, s.z], [before.x, before.y, before.z], 1)
                } else {
                    [0.0; 3]
                };
                t.last_seen = tick;
                t.pos = [s.x, s.y, s.z];
                t.yaw = s.yaw as u16;
                t.pitch = s.pitch as u8;
                t.held = s.held;
                t.lit = s.lit;
                t.wounded = s.wounded;
                t.dead = s.dead;
                t.sleeping = s.sleeping;
                t.crouched = s.crouched;
                t.aiming_at_me = t.species == Species::Player
                    && t.active()
                    && aimed_at(t.pos, t.crouched, t.yaw, t.pitch, chest);
            }
            let stale = if row.seen {
                !visible && tick.wrapping_sub(row.track.last_seen) >= FORGET_TICKS
            } else {
                !row.in_view
            };
            if stale {
                *slot = None;
            }
        }
    }

    /// Cast the most overdue sight lines, within the frame's ray budget.
    fn cast_rays(&mut self, core: &mut ClientCore, haven: &Haven, eye: [f32; 3], tick: u32) {
        let mut long = LONG_RAYS_PER_FRAME;
        let mut short = SHORT_RAYS_PER_FRAME;
        while long > 0 || short > 0 {
            let mut pick: Option<(usize, u32, bool)> = None;
            for (i, row) in self.rows.iter().enumerate() {
                let Some(row) = row.as_ref().filter(|r| r.in_view) else {
                    continue;
                };
                let s = row.sample;
                let d = ((s.x - eye[0]).powi(2) + (s.y - eye[1]).powi(2) + (s.z - eye[2]).powi(2))
                    .sqrt();
                let is_long = d > SHORT_RAY_M;
                if (is_long && long == 0) || (!is_long && short == 0) {
                    continue;
                }
                // A clear line is rechecked sooner than a blocked one: it is
                // what keeps a track in sight, and what a glimpse is timed on.
                let wait = if row.los_ok {
                    SPOT_TICKS
                } else {
                    LOS_CACHE_TICKS
                };
                // Most late past its own due tick first, so a backlog is
                // shared between clear and blocked lines.
                let overdue = match row.los_at {
                    None => u32::MAX,
                    Some(at) if tick.wrapping_sub(at) >= wait => tick.wrapping_sub(at) - wait,
                    Some(_) => continue,
                };
                if pick.is_none_or(|(_, o, _)| overdue > o) {
                    pick = Some((i, overdue, is_long));
                }
            }
            let Some((i, _, is_long)) = pick else {
                break;
            };
            if is_long {
                long -= 1;
            } else {
                short -= 1;
            }
            let Some(row) = self.rows[i].as_mut() else {
                break;
            };
            let s = row.sample;
            let to = [s.x, s.y + row.track.species.sight_y(s.crouched), s.z];
            self.stats.rays += 1;
            // `perceive` has range-checked it; the head sits above that.
            // A body standing in mine has nothing between us.
            let touching = (s.x - eye[0]).hypot(s.z - eye[2]) < TOUCH_M;
            let clear = touching
                || match clear_line(core, haven, eye, to, f32::INFINITY) {
                    Sight::Clear => true,
                    Sight::Terrain => {
                        self.stats.coarse_rejects += 1;
                        false
                    }
                    Sight::Blocked => false,
                };
            row.los_at = Some(tick);
            row.los_ok = clear;
            if !clear {
                row.los_since = None;
                continue;
            }
            let since = *row.los_since.get_or_insert(tick);
            if !row.seen && tick.wrapping_sub(since) >= SPOT_TICKS {
                row.seen = true;
                row.track.first_seen = tick;
                self.stats.spotted += 1;
            }
        }
    }

    fn slot_of(&self, id: u32) -> Option<usize> {
        self.rows
            .iter()
            .position(|r| r.as_ref().is_some_and(|r| r.track.id == id))
    }

    /// An empty row, else the track out of sight longest. A track in sight
    /// and a glimpse under way are never given up for a newcomer: with the
    /// table full of those the newcomer waits, rather than newcomers
    /// evicting each other every frame and none ever held long enough to
    /// be spotted.
    fn free_slot(&self, tick: u32) -> Option<usize> {
        if let Some(i) = self.rows.iter().position(Option::is_none) {
            return Some(i);
        }
        let mut worst = None;
        for (i, row) in self.rows.iter().enumerate() {
            let Some(r) = row.as_ref().filter(|r| r.seen && !r.track.visible) else {
                continue;
            };
            let age = tick.wrapping_sub(r.track.last_seen);
            if worst.is_none_or(|(_, a)| age > a) {
                worst = Some((i, age));
            }
        }
        worst.map(|(i, _)| i)
    }

    /// Every body seen, in sight or remembered.
    pub fn seen(&self) -> impl Iterator<Item = &Track> + '_ {
        self.rows
            .iter()
            .flatten()
            .filter(|r| r.seen)
            .map(|r| &r.track)
    }

    /// Bodies in view but not yet held long enough to be tracks (and not
    /// found behind something), where the snapshots put them: enough to
    /// keep a swing off them, not to act on.
    pub fn glimpses(&self) -> impl Iterator<Item = (u32, Species, [f32; 3])> + '_ {
        self.rows
            .iter()
            .flatten()
            .filter(|r| r.in_view && !r.seen && !r.sample.dead && (r.los_ok || r.los_at.is_none()))
            .map(|r| {
                let s = &r.sample;
                (r.track.id, r.track.species, [s.x, s.y, s.z])
            })
    }

    /// Bodies that can act, seen within `within` ticks of `tick`.
    pub fn recent(&self, tick: u32, within: u32) -> impl Iterator<Item = &Track> + '_ {
        self.seen()
            .filter(move |t| t.active() && tick.wrapping_sub(t.last_seen) < within)
    }

    /// A body in sight now that could be after me: a player or a wolf, up,
    /// and within `near_m`, aiming at me or closing on me. A pig grazing or
    /// a stranger far off walking past is not.
    pub fn pursuer_in_sight(&self, near_m: f32) -> bool {
        let Some(own) = self.own else {
            return false;
        };
        self.seen().any(|t| {
            if !t.visible || !t.active() || t.species == Species::Pig {
                return false;
            }
            let (dx, dz) = (own.pos[0] - t.pos[0], own.pos[2] - t.pos[2]);
            let d = (dx * dx + dz * dz).sqrt();
            d <= near_m || t.aiming_at_me || t.vel[0] * dx + t.vel[2] * dz >= CLOSING_MPS * d
        })
    }

    pub fn get(&self, id: u32) -> Option<&Track> {
        self.seen().find(|t| t.id == id)
    }

    /// Where a body in sight stands on my screen: its interpolated pose at
    /// the playout tick, the one the server judges melee and hitscan
    /// against. `None` for a body not in sight: nobody aims at a memory.
    pub fn aim_pose(&self, id: u32) -> Option<RemoteState> {
        self.get(id).filter(|t| t.visible)?;
        let at = f64::from(self.fed?) - f64::from(self.playout);
        let mut s = RemoteState::default();
        self.interp.sample(id, at, &mut s).then_some(s)
    }

    /// This body at the newest snapshot, and its velocity.
    pub fn own(&self) -> Option<Own> {
        self.own
    }

    /// Sounds heard, oldest first.
    pub fn heard(&self) -> impl Iterator<Item = &Heard> + '_ {
        (0..HEARD_ROWS).filter_map(move |k| self.heard[(self.heard_next + k) % HEARD_ROWS].as_ref())
    }

    /// Where the snapshots put a body when a sound came from it. Only ever
    /// turned into a bearing and a band, with noise.
    fn source(&self, id: u32) -> Option<[f32; 3]> {
        let mut s = RemoteState::default();
        self.interp
            .sample(id, f64::from(self.fed?), &mut s)
            .then_some([s.x, s.y, s.z])
    }

    fn hear(&mut self, sound: Sound, from: [f32; 3], max_m: f32, noisy: bool, tick: u32) {
        let Some(own) = self.own else {
            return;
        };
        let (dx, dz) = (from[0] - own.pos[0], from[2] - own.pos[2]);
        let mut d = (dx * dx + dz * dz).sqrt();
        if d > max_m {
            return;
        }
        let mut bearing = crate::agent::intent::yaw_toward(dx, dz);
        if noisy {
            let off = self.rng.next_bounded(2 * HEAR_BEARING_NOISE + 1);
            bearing = bearing
                .wrapping_add(off as u16)
                .wrapping_sub(HEAR_BEARING_NOISE as u16);
            d *= 0.75 + self.rng.next_bounded(51) as f32 / 100.0;
        }
        self.push_heard(Heard {
            sound,
            bearing,
            band: Band::of(d),
            tick,
        });
    }

    fn push_heard(&mut self, h: Heard) {
        self.heard[self.heard_next] = Some(h);
        self.heard_next = (self.heard_next + 1) % HEARD_ROWS;
        self.stats.heard += 1;
    }

    fn in_sight_mut(&mut self, id: u32) -> Option<&mut Track> {
        self.rows
            .iter_mut()
            .flatten()
            .find(|r| r.seen && r.track.visible && r.track.id == id)
            .map(|r| &mut r.track)
    }

    /// A shot was fired. Seen if its shooter is in sight; heard either way.
    pub fn on_shot(&mut self, shooter: u32, tick: u32) {
        if shooter == self.me {
            self.own_shot = Some(tick);
            return;
        }
        if let Some(t) = self.in_sight_mut(shooter) {
            t.last_shot = Some(tick);
        }
        if let Some(at) = self.source(shooter) {
            self.hear(Sound::Shot, at, SHOT_HEAR_M, true, tick);
        }
    }

    pub fn on_swing(&mut self, swinger: u32, tick: u32) {
        if swinger == self.me {
            return;
        }
        if let Some(t) = self.in_sight_mut(swinger) {
            t.last_swing = Some(tick);
        }
        if let Some(at) = self.source(swinger) {
            self.hear(Sound::Swing, at, SWING_HEAR_M, true, tick);
        }
    }

    /// An arrow stopped here: the mark is where it is, no guess needed.
    pub fn on_impact(&mut self, at: [f32; 3], tick: u32) {
        if self
            .own_shot
            .is_some_and(|t| tick.wrapping_sub(t) <= OWN_SHOT_QUIET_TICKS)
        {
            return;
        }
        self.hear(Sound::Impact, at, IMPACT_NEAR_M, false, tick);
    }

    /// A blow landed on me from this world bearing (a wire yaw).
    pub fn on_hurt(&mut self, toward: u16, tick: u32) {
        self.push_heard(Heard {
            sound: Sound::Hurt,
            bearing: toward,
            band: Band::Near,
            tick,
        });
    }

    pub fn on_death(&mut self, victim: u32, tick: u32) {
        if victim == self.me {
            return;
        }
        if let Some(t) = self.in_sight_mut(victim) {
            t.dead = true;
        }
        if let Some(at) = self.source(victim) {
            self.hear(Sound::Death, at, DEATH_HEAR_M, true, tick);
        }
    }

    /// My blow landed on this body (a hit marker).
    pub fn on_hit(&mut self, victim: u32, damage: u16) {
        if let Some(r) = self
            .rows
            .iter_mut()
            .flatten()
            .find(|r| r.seen && r.track.id == victim)
        {
            r.track.dealt = r.track.dealt.saturating_add(u32::from(damage));
        }
    }
}

/// Metres per second between two poses `ticks` apart; zero across a jump
/// no body could make.
fn velocity(now: [f32; 3], before: [f32; 3], ticks: u32) -> [f32; 3] {
    let k = TICK_HZ as f32 / ticks as f32;
    let v = [
        (now[0] - before[0]) * k,
        (now[1] - before[1]) * k,
        (now[2] - before[2]) * k,
    ];
    if v[0] * v[0] + v[1] * v[1] + v[2] * v[2] > MAX_BODY_MPS * MAX_BODY_MPS {
        [0.0; 3]
    } else {
        v
    }
}

/// After dusk, by the day clock the client reads (`weather::day_tick`).
/// The same numbers the sim's own test uses; restated here because the
/// agent may not name the sim's world module.
pub fn night(tick: u32, env: &Env) -> bool {
    let t = (weather::day_tick(u64::from(tick), env) + DAY_PHASE_TICKS) % DAY_TICKS;
    t as f32 / DAY_TICKS as f32 >= DAY_PORTION
}

/// Does a look ray from a body at `feet`, in that stance, pass within
/// [`AIM_MISS_M`] of `chest`, ahead of it?
fn aimed_at(feet: [f32; 3], crouched: bool, yaw: u16, pitch: u8, chest: [f32; 3]) -> bool {
    let eye_y = feet[1] + eye_m(crouched);
    let (fx, fz) = yaw_dir(yaw);
    let (h, v) = pitch_dir(pitch);
    let dir = [fx * h, v, fz * h];
    let to = [chest[0] - feet[0], chest[1] - eye_y, chest[2] - feet[2]];
    let along = to[0] * dir[0] + to[1] * dir[1] + to[2] * dir[2];
    if along <= 0.0 {
        return false;
    }
    let len2 = to[0] * to[0] + to[1] * to[1] + to[2] * to[2];
    len2 - along * along <= AIM_MISS_M * AIM_MISS_M
}

/// What a sight line met.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sight {
    Clear,
    /// The ground, found by the coarse pass.
    Terrain,
    /// Anything else: an occupant, a piece, a deployable, the fine walk's
    /// terrain, or a line longer than `max_m`.
    Blocked,
}

/// A line of sight, with the same terrain, occupant and built-volume
/// queries a player's projectile uses. Terrain is tried first at a coarse
/// spacing, which settles most hidden bodies for a few height lookups.
pub fn clear_line(
    core: &mut ClientCore,
    haven: &Haven,
    from: [f32; 3],
    to: [f32; 3],
    max_m: f32,
) -> Sight {
    let delta = [to[0] - from[0], to[1] - from[1], to[2] - from[2]];
    let length = (delta[0] * delta[0] + delta[1] * delta[1] + delta[2] * delta[2]).sqrt();
    if length > max_m {
        return Sight::Blocked;
    }
    let at = |t: f32| {
        [
            from[0] + delta[0] * t,
            from[1] + delta[1] * t,
            from[2] + delta[2] * t,
        ]
    };
    let (seed, mut island) = core.island();
    let coarse = (length / COARSE_STEP_M).ceil() as usize;
    for i in 1..coarse {
        let [x, y, z] = at(i as f32 / coarse as f32);
        if y < terrain::ground(seed, haven, x, z) {
            return Sight::Terrain;
        }
    }
    let steps = (length * MM_PER_M / ARROW_STEP_MM as f32).ceil() as usize;
    let point = |i: usize| at(i as f32 / steps.max(1) as f32);
    for i in 1..=steps {
        let [x, y, z] = point(i);
        if y <= terrain::ground(seed, haven, x, z) || island.blocks_volume(seed, x, z, y, 0.0, 0.0)
        {
            return Sight::Blocked;
        }
    }
    let mut prev = (from[0], from[2]);
    for i in 1..=steps {
        let [x, y, z] = point(i);
        if collide::shot_blocked(
            seed,
            haven,
            core.pieces.cols(),
            prev.0,
            prev.1,
            x,
            z,
            y,
            0.0,
        ) || collide::deploy_stop(seed, haven, core.pieces.cols(), x, z, y, 0.0).is_some()
        {
            return Sight::Blocked;
        }
        prev = (x, z);
    }
    Sight::Clear
}

#[cfg(test)]
impl Tracks {
    /// Put a body where the next snapshot has it, in sight or not, with
    /// no playout behind it: for tests of what aims at bodies.
    pub(crate) fn stand(&mut self, id: u32, pos: [f32; 3], visible: bool) {
        let tick = self.fed.map_or(1, |f| f + 1);
        let e = protocol::EntityState {
            id,
            qx: (pos[0] / POS_XZ_Q).round() as i32,
            qy: (pos[1] / POS_Y_Q).round() as i32,
            qz: (pos[2] / POS_XZ_Q).round() as i32,
            ..Default::default()
        };
        self.interp.push(tick, &e);
        self.fed = Some(tick);
        self.playout = 0;
        let i = self.slot_of(id).unwrap_or(0);
        let mut s = RemoteState::default();
        self.interp.sample(id, f64::from(tick), &mut s);
        let track = Track {
            id,
            species: Species::of(id),
            first_seen: 0,
            last_seen: tick,
            pos,
            vel: [0.0; 3],
            yaw: 0,
            pitch: 128,
            held: None,
            lit: false,
            wounded: false,
            dead: false,
            sleeping: false,
            crouched: false,
            dealt: 0,
            last_swing: None,
            last_shot: None,
            aiming_at_me: false,
            visible,
        };
        self.rows[i] = Some(Row {
            track,
            seen: true,
            in_view: visible,
            sample: s,
            los_at: Some(tick),
            los_ok: visible,
            los_since: None,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_look_ray_is_aimed_only_when_it_passes_close_ahead() {
        let feet = [0.0, 0.0, 0.0];
        let chest = |z: f32| [0.0, CHEST_Y_M, z];
        // Wire yaw 0 faces +Z; level pitch.
        let (fx, fz) = yaw_dir(0);
        assert!(fz > 0.9 && fx.abs() < 0.1);
        assert!(aimed_at(feet, false, 0, 128, chest(30.0)));
        assert!(!aimed_at(feet, false, 0, 128, chest(-30.0)), "behind it");
        assert!(!aimed_at(feet, false, 1 << 13, 128, chest(30.0)), "45° off");
        assert!(
            !aimed_at(feet, false, 0, 128, [3.0, CHEST_Y_M, 30.0]),
            "3 m wide"
        );
    }

    /// A seen track standing at `(x, z)`, in sight or not.
    fn row(id: u32, x: f32, z: f32, visible: bool, last_seen: u32) -> Option<Row> {
        Some(Row {
            track: Track {
                id,
                species: Species::of(id),
                first_seen: 0,
                last_seen,
                pos: [x, 0.0, z],
                vel: [0.0; 3],
                yaw: 0,
                pitch: 128,
                held: None,
                lit: false,
                wounded: false,
                dead: false,
                sleeping: false,
                crouched: false,
                dealt: 0,
                last_swing: None,
                last_shot: None,
                aiming_at_me: false,
                visible,
            },
            seen: true,
            in_view: visible,
            sample: RemoteState::default(),
            los_at: Some(last_seen),
            los_ok: visible,
            los_since: None,
        })
    }

    fn one(r: Option<Row>) -> Tracks {
        let mut t = Tracks::new();
        t.reset(1, 1);
        t.own = Some(Own::default());
        t.rows[0] = r;
        t
    }

    #[test]
    fn only_a_near_aiming_or_closing_player_or_wolf_is_a_pursuer() {
        let (wolf, pig) = (mob::mob_id(0), mob::mob_id(1));
        assert_eq!(
            (Species::of(wolf), Species::of(pig)),
            (Species::Wolf, Species::Pig)
        );
        assert!(one(row(2, 0.0, 20.0, true, 0)).pursuer_in_sight(32.0));
        assert!(one(row(wolf, 0.0, 20.0, true, 0)).pursuer_in_sight(32.0));
        assert!(!one(row(2, 0.0, 20.0, false, 0)).pursuer_in_sight(32.0));
        assert!(!one(row(pig, 0.0, 5.0, true, 0)).pursuer_in_sight(32.0));
        let mut far = one(row(2, 0.0, 80.0, true, 0));
        assert!(!far.pursuer_in_sight(32.0), "a stranger far off");
        far.rows[0].as_mut().unwrap().track.vel = [0.0, 0.0, -SPRINT_SPEED];
        assert!(far.pursuer_in_sight(32.0), "running at me");
        far.rows[0].as_mut().unwrap().track.vel = [SPRINT_SPEED, 0.0, 0.0];
        assert!(!far.pursuer_in_sight(32.0), "running past");
        far.rows[0].as_mut().unwrap().track.aiming_at_me = true;
        assert!(far.pursuer_in_sight(32.0), "aiming at me");
        far.rows[0].as_mut().unwrap().track.dead = true;
        assert!(!far.pursuer_in_sight(32.0));
    }

    #[test]
    fn a_full_table_gives_up_only_the_track_out_of_sight_longest() {
        let mut t = Tracks::new();
        for i in 0..TRACK_ROWS {
            t.rows[i] = row(10 + i as u32, 0.0, 10.0, true, 100);
        }
        assert_eq!(
            t.free_slot(100),
            None,
            "every row in sight: a newcomer waits"
        );
        t.rows[5] = row(5, 0.0, 10.0, false, 40);
        t.rows[9] = row(9, 0.0, 10.0, false, 20);
        assert_eq!(t.free_slot(100), Some(9));
    }

    #[test]
    fn my_own_arrow_landing_near_me_is_not_heard_as_being_shot_at() {
        let mut t = one(None);
        t.on_shot(1, 10);
        t.on_impact([1.0, 0.0, 1.0], 12);
        assert_eq!(t.heard().count(), 0);
        t.on_impact([1.0, 0.0, 1.0], 12 + OWN_SHOT_QUIET_TICKS + 1);
        assert_eq!(t.heard().map(|h| h.sound).last(), Some(Sound::Impact));
    }

    #[test]
    fn a_jump_no_body_could_make_has_no_velocity() {
        assert_eq!(
            velocity([0.1, 0.0, 0.0], [0.0; 3], 1)[0],
            0.1 * TICK_HZ as f32
        );
        assert_eq!(velocity([300.0, 0.0, 0.0], [0.0; 3], 1), [0.0; 3]);
    }

    #[test]
    fn heard_sounds_keep_the_newest_and_band_by_distance() {
        let mut t = Tracks::new();
        t.reset(1, 1);
        for i in 0..HEARD_ROWS as u32 + 3 {
            t.on_hurt(0, i);
        }
        let ticks: Vec<u32> = t.heard().map(|h| h.tick).collect();
        assert_eq!(ticks.len(), HEARD_ROWS);
        assert_eq!(ticks.first(), Some(&3));
        assert_eq!(ticks.last(), Some(&(HEARD_ROWS as u32 + 2)));
        assert_eq!(Band::of(5.0), Band::Near);
        assert_eq!(Band::of(40.0), Band::Mid);
        assert_eq!(Band::of(100.0), Band::Far);
    }
}
