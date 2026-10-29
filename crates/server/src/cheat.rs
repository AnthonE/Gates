//! Cheat checks: the per-player evidence an armed shard needs.
//!
//! The sim is authoritative, so the cheats that survive it are the ones
//! that only change what a client *knows* or how well it *aims*
//! (`DESIGN.md` §10's honest gaps: ESP inside the AOI, and aimbot). Neither
//! can be refused at the door, because every input such a client sends is
//! a legal one. What the shard can do is watch for the shape each leaves
//! in play and write it down, against the player, in the anomaly log.
//!
//! Three checks, each a count over a decaying window:
//!
//! 1. **Snap hits.** The view swung more than [`SNAP_DEG`] in the
//!    [`SNAP_TICKS`] before a shot, and that shot hit a player at range.
//!    A person flicks too; an aimbot flicks *and lands* nearly every time.
//! 2. **Headshot rate.** Heads over hits on players at range, once there
//!    are [`HEAD_MIN_HITS`] of them.
//! 3. **Wall tracking.** The crosshair stays on a player who is behind
//!    terrain or a built piece while that player *moves* and the aim
//!    *follows*. Holding an angle on where somebody ducked out of sight is
//!    normal, and so is following them on past it the way they were going;
//!    following them through a hill long after, somewhere the last sight
//!    of them would not put them, is ESP.
//!
//! A hit on a body that was already lying down (asleep or wounded) counts
//! toward neither rate: it holds still for a head and needs no flick.
//!
//! **Evidence, never a verdict.** A check flags; it does not kick or ban.
//! Bans are the operator's (`CLAUDE.md`), and every threshold here is set
//! so an honest good player should almost never cross it, which is the
//! trade a log that someone reads wants. Signed agents are exempt: they
//! are declared programs (`DECISIONS.md` 2026-08-01) and do not play on
//! armed shards anyway.
//!
//! Runs on the sim thread after `World::tick`, allocation-free: fixed
//! per-slot state, the tick's own events, and a capped number of sight
//! lines per tick ([`WALL_RAYS_PER_TICK`]).

use sim_core::collide::{self, ColIndex, Part};
use sim_core::limits::{ARROW_STEP_MM, MAX_PLAYERS, MOB_ID_TAG, TICK_HZ};
use sim_core::movement::{POS_XZ_Q, POS_Y_Q};
use sim_core::ranged::{self, MM_PER_M};
use sim_core::terrain::{self, Haven, Lattice};
use sim_core::world::{hit_part, Player, SimEvent, EV_HIT, EV_SHOT, STRUCT_VICTIM};

/// The window the counts decay over: every this many ticks each count
/// halves and a check may flag the same player again. Five minutes.
pub const WINDOW_TICKS: u64 = 5 * 60 * TICK_HZ as u64;

/// A hit closer than this is not counted: melee, and the point-blank
/// shots where a flick and a head are both easy.
pub const HIT_MIN_M: f32 = 20.0;

/// How far back a snap is measured from the shot's tick: 100 ms.
pub const SNAP_TICKS: u64 = 3;
/// The swing that makes a shot a snap: 30° in 100 ms is 300°/s, landed.
pub const SNAP_DEG: f32 = 30.0;
/// How long a shot's snap waits for its hit: an arrow's flight.
pub const SNAP_HIT_TICKS: u64 = 4 * TICK_HZ as u64;
/// Snap hits in a window before the check flags…
pub const SNAP_MIN: u32 = 4;
/// …and the share of ranged hits they must be, percent.
pub const SNAP_MIN_PCT: u32 = 25;

/// Ranged hits on players in a window before the headshot rate counts.
pub const HEAD_MIN_HITS: u32 = 20;
/// The headshot share that flags, percent.
pub const HEAD_MIN_PCT: u32 = 60;

/// How often the wall check looks, ticks.
pub const WALL_CADENCE: u64 = 3;
/// Targets nearer than this are left out (a body you are standing on) and
/// further than this too (the sight line's cost).
pub const WALL_MIN_M: f32 = 4.0;
pub const WALL_MAX_M: f32 = 100.0;
/// How close to a hidden body the aim must stay, degrees — widened at
/// short range to the body's own width.
pub const WALL_CONE_DEG: f32 = 3.0;
/// One wall track: this long on one hidden target…
pub const WALL_TRACK_TICKS: u32 = 2 * TICK_HZ;
/// …while the target moves at least this far…
pub const WALL_MOVE_M: f32 = 4.0;
/// …and the aim turns at least this much following it.
pub const WALL_SWEEP_DEG: f32 = 8.0;
/// A body out of sight is where a fight around cover expects it for this
/// long: held, or pre-aimed at its pop-out. No look on it counts until then.
pub const WALL_GRACE_TICKS: u64 = 2 * TICK_HZ as u64;
/// A look counts only with the body at least this far from where the
/// watcher's last sight of it, carried on at its speed then, puts it.
pub const WALL_NEAR_M: f32 = 5.0;
/// Wall tracks in a window before the check flags.
pub const WALL_MIN: u32 = 3;
/// Sight lines the wall check may cast per look.
pub const WALL_RAYS_PER_TICK: usize = 16;
/// The longest gap between two looks at one watcher that still grows its
/// track: a crowded shard looks at each watcher less often.
pub const WALL_GAP_TICKS: u64 = TICK_HZ as u64 / 2;

/// Coarse terrain step of a sight line, metres: a hill, not a curb.
const SIGHT_COARSE_M: f32 = 2.0;
/// Chest and head above the feet, metres.
const CHEST_M: f32 = 1.1;
const HEAD_M: f32 = 1.65;
/// The aim history's length, ticks.
const AIM_HIST: usize = 8;

/// Which check flagged. The anomaly record's `what`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u16)]
pub enum Check {
    /// `a` snap hits of `b` ranged hits; `c` the widest snap, degrees.
    Snap = 0,
    /// `a` heads of `b` ranged hits; `c` the percent.
    Head = 1,
    /// `a` wall tracks; `b` the last target's id; `c` its range, metres.
    Wall = 2,
}

impl Check {
    pub fn name(self) -> &'static str {
        match self {
            Check::Snap => "snap",
            Check::Head => "headshots",
            Check::Wall => "wall_track",
        }
    }

    pub fn from_code(code: u16) -> Option<Check> {
        Some(match code {
            0 => Check::Snap,
            1 => Check::Head,
            2 => Check::Wall,
            _ => return None,
        })
    }

    fn bit(self) -> u8 {
        1 << self as u8
    }
}

/// One flag, for the caller to log.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Flag {
    pub who: u32,
    pub check: Check,
    pub a: i64,
    pub b: i64,
    pub c: i64,
}

type V3 = (f32, f32, f32);

#[derive(Clone, Copy, Default)]
struct Slot {
    id: u32,
    /// Unit aim per tick, `tick % AIM_HIST`, stamped with its tick.
    aim: [(u64, V3); AIM_HIST],
    shot_tick: u64,
    shot_snap: f32,
    last_hit_tick: u64,
    hits: u32,
    heads: u32,
    snaps: u32,
    widest_snap: f32,
    walls: u32,
    /// Checks already flagged this window.
    flagged: u8,
    /// Asleep or wounded at the last tick's end: what a hit this tick found.
    down: bool,
    track: Track,
    /// The last body this watcher had in its crosshair in the open.
    open: Sighting,
}

#[derive(Clone, Copy, Default)]
struct Sighting {
    target: u32,
    tick: u64,
    at: V3,
    /// Metres per tick, from the look before.
    vel: V3,
}

impl Sighting {
    /// Whether a body at `at` on `tick` is somewhere this sight of it would
    /// not put it: another body, or off its line by [`WALL_NEAR_M`].
    fn surprised(&self, target: u32, at: V3, tick: u64) -> bool {
        let dt = tick.saturating_sub(self.tick) as f32;
        let guess = (
            self.at.0 + self.vel.0 * dt,
            self.at.1 + self.vel.1 * dt,
            self.at.2 + self.vel.2 * dt,
        );
        self.target != target || len(sub(at, guess)) >= WALL_NEAR_M
    }
}

#[derive(Clone, Copy, Default)]
struct Track {
    target: u32,
    /// The look the body was first hidden on.
    hid: u64,
    /// The look this track was last grown on.
    seen: u64,
    ticks: u32,
    moved: f32,
    swept: f32,
    at: V3,
    aim: V3,
}

/// Every slot's checks. One per shard, beside the anomaly sink.
pub struct Checks {
    slots: Box<[Slot]>,
    /// Where the wall check's next look starts, so a ray budget that runs
    /// out reaches every watcher in turn.
    next: usize,
}

impl Default for Checks {
    fn default() -> Self {
        Self::new()
    }
}

/// The world a sight line is walked through.
pub struct Ground<'a> {
    pub seed: u64,
    pub haven: &'a Haven,
    pub cols: &'a ColIndex,
    /// The worldgen draw's memo: a sight line is one locality.
    pub lat: &'a mut Lattice,
}

impl Ground<'_> {
    /// Whether terrain or a built piece stands between `from` and `to`
    /// (metres). Trees, rocks and bodies are not asked: a check that errs
    /// toward "visible" accuses nobody it should not.
    pub fn hidden(&mut self, from: V3, to: V3) -> bool {
        let d = (to.0 - from.0, to.1 - from.1, to.2 - from.2);
        let len = (d.0 * d.0 + d.1 * d.1 + d.2 * d.2).sqrt();
        let at = |t: f32| (from.0 + d.0 * t, from.1 + d.1 * t, from.2 + d.2 * t);
        let coarse = (len / SIGHT_COARSE_M).ceil() as usize;
        for i in 1..coarse {
            let (x, y, z) = at(i as f32 / coarse as f32);
            if y < terrain::ground_memo(self.lat, self.seed, self.haven, x, z) {
                return true;
            }
        }
        let steps = ((len * MM_PER_M / ARROW_STEP_MM as f32).ceil() as usize).max(1);
        let mut prev = (from.0, from.2);
        for i in 1..=steps {
            let (x, y, z) = at(i as f32 / steps as f32);
            if collide::shot_blocked(
                self.seed, self.haven, self.cols, prev.0, prev.1, x, z, y, 0.0,
            ) {
                return true;
            }
            prev = (x, z);
        }
        false
    }
}

fn feet(p: &Player) -> V3 {
    (
        p.body.qx as f32 * POS_XZ_Q,
        p.body.qy as f32 * POS_Y_Q,
        p.body.qz as f32 * POS_XZ_Q,
    )
}

fn sub(a: V3, b: V3) -> V3 {
    (a.0 - b.0, a.1 - b.1, a.2 - b.2)
}

fn len(a: V3) -> f32 {
    (a.0 * a.0 + a.1 * a.1 + a.2 * a.2).sqrt()
}

/// Degrees between two directions (either length).
fn angle_deg(a: V3, b: V3) -> f32 {
    let n = len(a) * len(b);
    if n <= 0.0 {
        return 0.0;
    }
    let c = ((a.0 * b.0 + a.1 * b.1 + a.2 * b.2) / n).clamp(-1.0, 1.0);
    c.acos().to_degrees()
}

fn aim_of(p: &Player) -> V3 {
    ranged::aim_dir(p.frame.yaw, p.frame.pitch, p.frame.pitch_fine)
}

/// A body that can be in a fight: here, alive, awake.
fn standing(p: &Player) -> bool {
    p.active && !p.dead && !p.sleeping
}

fn slot_of(players: &[Player; MAX_PLAYERS], id: u32) -> Option<usize> {
    if id == 0 || id & MOB_ID_TAG != 0 || id == STRUCT_VICTIM {
        return None;
    }
    players.iter().position(|p| p.active && p.id == id)
}

impl Checks {
    pub fn new() -> Self {
        Self {
            slots: vec![Slot::default(); MAX_PLAYERS].into_boxed_slice(),
            next: 0,
        }
    }

    /// One tick, after `World::tick`: read every body's aim, this tick's
    /// shots and hits, and (on the cadence) the wall check. `exempt` names
    /// players no check applies to; `mates` whether two are on one team;
    /// `hidden` is the sight line ([`Ground::hidden`] on a shard).
    #[allow(clippy::too_many_arguments)]
    pub fn tick(
        &mut self,
        tick: u64,
        players: &[Player; MAX_PLAYERS],
        events: &[SimEvent],
        exempt: impl Fn(u32) -> bool,
        mates: impl Fn(u32, u32) -> bool,
        mut hidden: impl FnMut(V3, V3) -> bool,
        mut flag: impl FnMut(Flag),
    ) {
        let decay = tick.is_multiple_of(WINDOW_TICKS);
        for (w, p) in players.iter().enumerate() {
            let s = &mut self.slots[w];
            if !p.active {
                s.id = 0;
                continue;
            }
            if s.id != p.id {
                *s = Slot {
                    id: p.id,
                    ..Slot::default()
                };
            }
            s.aim[tick as usize % AIM_HIST] = (tick, aim_of(p));
            if decay {
                s.hits /= 2;
                s.heads /= 2;
                s.snaps /= 2;
                s.walls /= 2;
                s.widest_snap = 0.0;
                s.flagged = 0;
            }
        }

        for ev in events {
            match ev.code {
                EV_SHOT => {
                    let Some(w) = slot_of(players, ev.a) else {
                        continue;
                    };
                    let s = &mut self.slots[w];
                    let now = s.aim[tick as usize % AIM_HIST];
                    let then = s.aim[(tick.wrapping_sub(SNAP_TICKS)) as usize % AIM_HIST];
                    s.shot_tick = tick;
                    s.shot_snap = if now.0 == tick && then.0 == tick.wrapping_sub(SNAP_TICKS) {
                        angle_deg(now.1, then.1)
                    } else {
                        0.0
                    };
                }
                EV_HIT => {
                    let (Some(w), Some(v)) = (slot_of(players, ev.a), slot_of(players, ev.b))
                    else {
                        continue;
                    };
                    if w == v || exempt(ev.a) || mates(ev.a, ev.b) || self.slots[v].down {
                        continue;
                    }
                    let range = len(sub(feet(&players[v]), feet(&players[w])));
                    let s = &mut self.slots[w];
                    // One hit per shooter per tick: a shotgun's pellets are
                    // one shot.
                    if range < HIT_MIN_M || (s.last_hit_tick == tick && s.hits > 0) {
                        continue;
                    }
                    s.last_hit_tick = tick;
                    s.hits += 1;
                    if hit_part(ev.c) == Part::Head {
                        s.heads += 1;
                    }
                    if tick.saturating_sub(s.shot_tick) <= SNAP_HIT_TICKS && s.shot_snap >= SNAP_DEG
                    {
                        s.snaps += 1;
                        s.widest_snap = s.widest_snap.max(s.shot_snap);
                        // Spent: the arrow's second victim is no second snap.
                        s.shot_snap = 0.0;
                    }
                    Self::judge_aim(s, &mut flag);
                }
                _ => {}
            }
        }
        for (s, p) in self.slots.iter_mut().zip(players.iter()) {
            s.down = p.sleeping || p.wounded;
        }

        if tick.is_multiple_of(WALL_CADENCE) {
            self.walls(tick, players, &exempt, &mates, &mut hidden, &mut flag);
        }
    }

    fn judge_aim(s: &mut Slot, flag: &mut impl FnMut(Flag)) {
        if s.flagged & Check::Snap.bit() == 0
            && s.snaps >= SNAP_MIN
            && s.snaps * 100 >= SNAP_MIN_PCT * s.hits
        {
            s.flagged |= Check::Snap.bit();
            flag(Flag {
                who: s.id,
                check: Check::Snap,
                a: s.snaps.into(),
                b: s.hits.into(),
                c: s.widest_snap as i64,
            });
        }
        if s.flagged & Check::Head.bit() == 0
            && s.hits >= HEAD_MIN_HITS
            && s.heads * 100 >= HEAD_MIN_PCT * s.hits
        {
            s.flagged |= Check::Head.bit();
            flag(Flag {
                who: s.id,
                check: Check::Head,
                a: s.heads.into(),
                b: s.hits.into(),
                c: (s.heads * 100 / s.hits).into(),
            });
        }
    }

    /// The wall check: for each watcher, the body nearest its crosshair; if
    /// that body is hidden, the track on it grows.
    fn walls(
        &mut self,
        tick: u64,
        players: &[Player; MAX_PLAYERS],
        exempt: &impl Fn(u32) -> bool,
        mates: &impl Fn(u32, u32) -> bool,
        hidden: &mut impl FnMut(V3, V3) -> bool,
        flag: &mut impl FnMut(Flag),
    ) {
        let mut rays = 0usize;
        let start = self.next;
        for k in 0..MAX_PLAYERS {
            let w = (start + k) % MAX_PLAYERS;
            let a = &players[w];
            if !standing(a) || exempt(a.id) {
                self.slots[w].track = Track::default();
                continue;
            }
            let eye = ranged_eye(a);
            let aim = aim_of(a);
            let mut best: Option<(usize, f32, f32)> = None;
            for (v, b) in players.iter().enumerate() {
                if v == w || !b.active || b.dead || mates(a.id, b.id) {
                    continue;
                }
                let chest = add_y(feet(b), CHEST_M);
                let to = sub(chest, eye);
                let d = len(to);
                if !(WALL_MIN_M..=WALL_MAX_M).contains(&d) {
                    continue;
                }
                let off = angle_deg(aim, to);
                let cone = WALL_CONE_DEG.max((0.6 / d).atan().to_degrees());
                if off <= cone && best.is_none_or(|(_, o, _)| off < o) {
                    best = Some((v, off, d));
                }
            }
            let Some((v, _, d)) = best else {
                self.slots[w].track = Track::default();
                continue;
            };
            if rays + 2 > WALL_RAYS_PER_TICK {
                // Out of budget: this watcher is first next look, and its
                // track holds rather than resets.
                self.next = w;
                return;
            }
            let b = &players[v];
            let foot = feet(b);
            rays += 2;
            let unseen = hidden(eye, add_y(foot, CHEST_M)) && hidden(eye, add_y(foot, HEAD_M));
            let s = &mut self.slots[w];
            if !unseen {
                s.track = Track::default();
                let o = &mut s.open;
                let dt = tick - o.tick;
                o.vel = if o.target == b.id && (1..=WALL_GAP_TICKS).contains(&dt) {
                    let d = sub(foot, o.at);
                    let dt = dt as f32;
                    (d.0 / dt, d.1 / dt, d.2 / dt)
                } else {
                    (0.0, 0.0, 0.0)
                };
                o.target = b.id;
                o.tick = tick;
                o.at = foot;
                continue;
            }
            let t = &mut s.track;
            // A watcher the budget skipped for long is a fresh look.
            if t.target != b.id || tick - t.seen > WALL_GAP_TICKS {
                *t = Track {
                    target: b.id,
                    hid: tick,
                    seen: tick,
                    ticks: 0,
                    moved: 0.0,
                    swept: 0.0,
                    at: foot,
                    aim,
                };
                continue;
            }
            let (dt, step, turn) = (tick - t.seen, len(sub(foot, t.at)), angle_deg(aim, t.aim));
            t.seen = tick;
            t.at = foot;
            t.aim = aim;
            // Holding or pre-aiming where a body went is a fight around
            // cover: only a look past the grace, on a body somewhere the
            // last sight of it would not put it, grows the track.
            if tick - t.hid < WALL_GRACE_TICKS || !s.open.surprised(b.id, foot, tick) {
                continue;
            }
            t.ticks += dt as u32;
            t.moved += step;
            t.swept += turn;
            if t.ticks >= WALL_TRACK_TICKS {
                if t.moved >= WALL_MOVE_M && t.swept >= WALL_SWEEP_DEG {
                    s.walls += 1;
                    if s.flagged & Check::Wall.bit() == 0 && s.walls >= WALL_MIN {
                        s.flagged |= Check::Wall.bit();
                        flag(Flag {
                            who: s.id,
                            check: Check::Wall,
                            a: s.walls.into(),
                            b: b.id.into(),
                            c: d as i64,
                        });
                    }
                }
                let t = &mut s.track;
                t.ticks = 0;
                t.moved = 0.0;
                t.swept = 0.0;
            }
        }
        self.next = (start + 1) % MAX_PLAYERS;
    }
}

fn add_y(a: V3, dy: f32) -> V3 {
    (a.0, a.1 + dy, a.2)
}

/// The eye every shot leaves from, metres.
fn ranged_eye(p: &Player) -> V3 {
    add_y(feet(p), ranged::eye_mm(p.crouched()) as f32 / MM_PER_M)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_core::world::hit_c;

    fn player(id: u32, x: f32, z: f32) -> Player {
        let mut p = Player {
            id,
            active: true,
            ..Player::default()
        };
        p.body.qx = (x / POS_XZ_Q) as i32;
        p.body.qz = (z / POS_XZ_Q) as i32;
        p
    }

    /// The frame that aims `from` at `at`'s chest.
    fn aim_at(from: &mut Player, at: V3) {
        let eye = ranged_eye(from);
        let d = sub(at, eye);
        let (yaw, pitch, fine) = ranged::wire_angles({
            let l = len(d);
            (d.0 / l, d.1 / l, d.2 / l)
        });
        from.frame.yaw = yaw;
        from.frame.pitch = pitch;
        from.frame.pitch_fine = fine;
    }

    fn field() -> Box<[Player; MAX_PLAYERS]> {
        Box::new([Player::default(); MAX_PLAYERS])
    }

    fn run(
        c: &mut Checks,
        tick: u64,
        ps: &[Player; MAX_PLAYERS],
        evs: &[SimEvent],
        hidden: bool,
    ) -> Vec<Flag> {
        let mut out = Vec::new();
        c.tick(
            tick,
            ps,
            evs,
            |_| false,
            |_, _| false,
            |_, _| hidden,
            |f| out.push(f),
        );
        out
    }

    fn ev(code: u8, a: u32, b: u32, c: u32) -> SimEvent {
        SimEvent { code, a, b, c }
    }

    /// A shooter that turns 90° in one tick and hits, again and again, is
    /// flagged once per window, and one that hits without swinging is not.
    #[test]
    fn snap_hits_flag_and_steady_hits_do_not() {
        for snap in [true, false] {
            let mut c = Checks::new();
            let mut ps = field();
            ps[0] = player(1, 0.0, 0.0);
            ps[1] = player(2, 0.0, 40.0);
            let target = add_y(feet(&ps[1]), CHEST_M);
            let mut flags = Vec::new();
            let mut tick = 1;
            for _ in 0..8 {
                // Look away, then (if snapping) come back in one tick.
                for _ in 0..4 {
                    if snap {
                        aim_at(&mut ps[0], (40.0, 1.5, 0.0));
                    } else {
                        aim_at(&mut ps[0], target);
                    }
                    flags.extend(run(&mut c, tick, &ps, &[], false));
                    tick += 1;
                }
                aim_at(&mut ps[0], target);
                let head = hit_c(Part::Chest, 30);
                flags.extend(run(
                    &mut c,
                    tick,
                    &ps,
                    &[ev(EV_SHOT, 1, 0, 0), ev(EV_HIT, 1, 2, head)],
                    false,
                ));
                tick += 1;
            }
            let snaps: Vec<_> = flags.iter().filter(|f| f.check == Check::Snap).collect();
            if snap {
                assert_eq!(snaps.len(), 1, "{flags:?}");
                assert_eq!(snaps[0].who, 1);
                assert!(snaps[0].c >= 80, "widest snap {}", snaps[0].c);
            } else {
                assert!(snaps.is_empty(), "{flags:?}");
            }
        }
    }

    #[test]
    fn a_headshot_rate_flags_only_past_its_sample_floor() {
        let mut c = Checks::new();
        let mut ps = field();
        ps[0] = player(1, 0.0, 0.0);
        ps[1] = player(2, 0.0, 30.0);
        let mut flags = Vec::new();
        for i in 0..HEAD_MIN_HITS as u64 {
            let part = if i % 4 == 3 { Part::Chest } else { Part::Head };
            let evs = [ev(EV_HIT, 1, 2, hit_c(part, 30))];
            flags.extend(run(&mut c, 10 + i * 10, &ps, &evs, false));
            if i + 1 < HEAD_MIN_HITS as u64 {
                assert!(flags.is_empty(), "flagged on {} hits", i + 1);
            }
        }
        assert_eq!(flags.len(), 1);
        assert_eq!(flags[0].check, Check::Head);
        assert_eq!((flags[0].a, flags[0].b, flags[0].c), (15, 20, 75));
        // Close hits never count.
        let mut c = Checks::new();
        ps[1] = player(2, 0.0, 5.0);
        for i in 0..40 {
            let evs = [ev(EV_HIT, 1, 2, hit_c(Part::Head, 30))];
            assert!(run(&mut c, 10 + i * 10, &ps, &evs, false).is_empty());
        }
    }

    /// Following a moving body that is behind a wall is a track; following
    /// the same body in the open is not, and neither is holding still on a
    /// hidden one.
    #[test]
    fn following_a_hidden_body_is_a_wall_track() {
        for (hidden, moves, flags_want) in [(true, true, 1), (false, true, 0), (true, false, 0)] {
            let mut c = Checks::new();
            let mut ps = field();
            ps[0] = player(1, 0.0, 0.0);
            let mut flags = Vec::new();
            let looks =
                (WALL_GRACE_TICKS + WALL_TRACK_TICKS as u64 * (WALL_MIN as u64 + 1)) / WALL_CADENCE;
            for k in 0..looks {
                let x = if moves { -15.0 + k as f32 * 0.3 } else { 0.0 };
                ps[1] = player(2, x, 30.0);
                let chest = add_y(feet(&ps[1]), CHEST_M);
                aim_at(&mut ps[0], chest);
                flags.extend(run(&mut c, k * WALL_CADENCE, &ps, &[], hidden));
            }
            let walls = flags.iter().filter(|f| f.check == Check::Wall).count();
            assert_eq!(
                walls, flags_want,
                "hidden {hidden} moves {moves}: {flags:?}"
            );
        }
    }

    /// A fight around cover: the body runs on behind a wall the way it was
    /// going and the aim carries on with it to its pop-out, again and again,
    /// and that is no track. Turning back once out of sight, past the grace,
    /// and being followed there still is.
    #[test]
    fn following_a_body_on_past_cover_is_not_a_wall_track() {
        for (turns, flags_want) in [(false, 0), (true, 1)] {
            let mut c = Checks::new();
            let mut ps = field();
            ps[0] = player(1, 0.0, 0.0);
            ps[1] = player(2, -10.0, 30.0);
            let mut flags = Vec::new();
            let mut tick = 0;
            // Quanta per look: 0.3 m, a walk.
            let mut dir = 10;
            for _ in 0..6 {
                for look in 0..120 {
                    if turns && look == 40 {
                        dir = -dir;
                    }
                    ps[1].body.qx += dir;
                    let chest = add_y(feet(&ps[1]), CHEST_M);
                    aim_at(&mut ps[0], chest);
                    flags.extend(run(&mut c, tick, &ps, &[], look >= 10));
                    tick += WALL_CADENCE;
                }
                dir = -dir;
            }
            let walls = flags.iter().filter(|f| f.check == Check::Wall).count();
            assert_eq!(walls, flags_want, "turns {turns}: {flags:?}");
        }
    }

    /// A body already lying down, asleep or wounded, holds still for a head
    /// and needs no flick: snapped head hits on it count toward neither
    /// rate. The hit that put it down does.
    #[test]
    fn hits_on_lying_bodies_count_toward_no_rate() {
        for (sleeping, wounded, flagged) in [
            (true, false, false),
            (false, true, false),
            (false, false, true),
        ] {
            let mut c = Checks::new();
            let mut ps = field();
            ps[0] = player(1, 0.0, 0.0);
            ps[1] = player(2, 0.0, 40.0);
            ps[1].sleeping = sleeping;
            ps[1].wounded = wounded;
            let target = add_y(feet(&ps[1]), CHEST_M);
            let mut flags = run(&mut c, 1, &ps, &[], false);
            for i in 0..40u64 {
                let t = 10 + i * 10;
                aim_at(&mut ps[0], (40.0, 1.5, 0.0));
                flags.extend(run(&mut c, t - SNAP_TICKS, &ps, &[], false));
                aim_at(&mut ps[0], target);
                // The knock: down only from the hit's own tick.
                ps[1].wounded |= flagged;
                let evs = [
                    ev(EV_SHOT, 1, 0, 0),
                    ev(EV_HIT, 1, 2, hit_c(Part::Head, 30)),
                ];
                flags.extend(run(&mut c, t, &ps, &evs, false));
                ps[1].wounded = wounded;
            }
            let checks: Vec<_> = flags.iter().map(|f| f.check).collect();
            if flagged {
                assert!(
                    checks.contains(&Check::Head) && checks.contains(&Check::Snap),
                    "{flags:?}"
                );
            } else {
                assert!(checks.is_empty(), "{flags:?}");
            }
        }
    }

    #[test]
    fn agents_and_mates_are_exempt() {
        let mut ps = field();
        ps[0] = player(1, 0.0, 0.0);
        ps[1] = player(2, 0.0, 30.0);
        for (agent, mate) in [(true, false), (false, true)] {
            let mut c = Checks::new();
            let mut n = 0;
            for i in 0..40u64 {
                let evs = [ev(EV_HIT, 1, 2, hit_c(Part::Head, 30))];
                c.tick(
                    10 + i * 10,
                    &ps,
                    &evs,
                    |id| agent && id == 1,
                    |_, _| mate,
                    |_, _| false,
                    |_| n += 1,
                );
            }
            assert_eq!(n, 0);
        }
    }
}

#[cfg(test)]
mod sight {
    use super::*;

    /// A line under the island is hidden and one far above it is not.
    #[test]
    fn the_ground_hides_and_the_sky_does_not() {
        let seed = 7;
        let haven = terrain::haven(seed);
        let cols = ColIndex::default();
        let mut lat = Lattice::new();
        let mut g = Ground {
            seed,
            haven: &haven,
            cols: &cols,
            lat: &mut lat,
        };
        let t = std::time::Instant::now();
        let mut n = 0;
        for k in 0..WALL_RAYS_PER_TICK {
            let x = k as f32 * 10.0;
            n += usize::from(g.hidden((x, -500.0, 0.0), (x + 60.0, -500.0, 80.0)));
            assert!(!g.hidden((x, 900.0, 0.0), (x + 60.0, 900.0, 80.0)));
        }
        assert_eq!(n, WALL_RAYS_PER_TICK);
        eprintln!("{} sight lines: {:?}", 2 * WALL_RAYS_PER_TICK, t.elapsed());
    }
}
