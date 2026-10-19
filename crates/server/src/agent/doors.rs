//! Working a teammate's doors as a player works them: one at a time,
//! walked up to square on, opened with `E`, walked straight through and
//! shut behind ([`Doors`]). A lock that does not know it takes the code a
//! teammate gave (an external agent's `"code"`, kept as a [`LockCode`]),
//! entered once at its keypad. The route's metre grid has no room by a
//! door for what stands round it, so the last steps are walked, stepping
//! round what is in the way.
//!
//! Which door: the nearest one near where it is going, when the route says
//! there is no way there ([`nearest_door`]); the nearest one after a
//! teammate on the other side of a wall ([`walled_between`],
//! [`Leave::after`]); and out of a teammate's base before a walk into the
//! island, walled in on a built floor ([`enclosed`], [`Leave::out`]).

use crate::agent::build::{aim_point, door_open, e_picks_by, look_at, PRESS_SLACK_M};
use crate::agent::hands::Hands;
use crate::agent::home::HOLD_TICKS;
use crate::agent::intent::{yaw_toward, Intent, Look};
use crate::agent::lock::LockCode;
use crate::agent::route::{Route, Step};
use crate::agent::tracks::{clear_line, Sight};
use crate::mind::Why;
use client_core::core::ClientCore;
use protocol::EntityState;
use sim_core::bots::OpAddr;
use sim_core::build::{
    anchor, build_cell_of, BUILD_CELL_M, LOC_DIAG_A, LOC_DIAG_B, LOC_EDGE_XLO, LOC_EDGE_ZLO,
    SHAPE_DOORWAY, SHAPE_FRAME,
};
use sim_core::deploy::{
    ARCH_DOOR, ARCH_GARAGE_DOOR, REFUSE_D_AUTH_FULL, REFUSE_D_LOCKOUT, REFUSE_D_OWNER,
};
use sim_core::limits::TICK_HZ;
use sim_core::movement::{POS_XZ_Q, POS_Y_Q};
use sim_core::terrain::Haven;
use sim_core::yaw_dir;

/// Doors this near the chest are the ones in the way to it.
pub const HOP_M: f32 = 12.0;
/// Where a door is worked from: this far out on its doorway's middle
/// line, square in front of it (an open leaf does not stand in the way)...
pub const HOP_STAND_M: f32 = 0.8;
/// ...within this of that spot. A route that gives up nearer than
/// [`HOP_STRAIGHT_M`] to it walks the rest straight.
pub const HOP_NEAR_M: f32 = 0.6;
pub const HOP_STRAIGHT_M: f32 = 3.0;
/// The walk through lines up on the doorway's middle this far out, to
/// within [`HOP_LINE_M`] of it, then walks straight through...
pub const HOP_LINE_OUT_M: f32 = 0.45;
pub const HOP_LINE_M: f32 = 0.15;
/// ...and is through once this far past it, still in line with it.
pub const HOP_PAST_M: f32 = 0.6;
/// A straight walk that has come no closer by this much in this long has
/// something in its way, and side-steps it this long.
pub const DODGE_PROGRESS_M: f32 = 0.2;
pub const DODGE_AFTER_TICKS: u32 = TICK_HZ;
pub const DODGE_TICKS: u32 = TICK_HZ * 2 / 3;
/// One stage of a door (the walk up, the press, the walk through, the
/// shut) that has not finished in this long is given up.
pub const HOP_STAGE_TICKS: u32 = 15 * TICK_HZ;
/// Closer than its best by this much is progress on the walk up, which
/// restarts that stage's clock.
const HOP_PROGRESS_M: f32 = 0.5;
/// Doors one delivery goes through on the way in, at most.
pub const MAX_DOORS: usize = 4;
/// Presses that came to nothing before the delivery gives up.
pub const MAX_TRIES: u8 = 6;
/// The way out of a teammate's base is a door this near...
pub const LEAVE_M: f32 = 8.0;
/// ...and walled in is a wall this near every way.
pub const ENCLOSED_M: f32 = 6.0;
/// A way out or through that came to nothing is not tried again for this
/// long.
pub const LEAVE_RETRY_TICKS: u32 = 30 * TICK_HZ;
/// A follow does not go back through the door it just came through for
/// this long.
pub const BACK_THROUGH_TICKS: u32 = 10 * TICK_HZ;
/// An answer to a press comes within this long.
const VERDICT_TICKS: u32 = 3 * TICK_HZ;

/// What a walk through a door wants next. `explorer.rs` sends the verbs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Through {
    Go(Intent),
    /// Press use on the door at this address.
    Use(OpAddr, Intent),
    /// Enter the team's code at the lock at this address.
    Enter(OpAddr, LockCode, Intent),
    /// Through this door, and it is shut behind.
    Passed(OpAddr),
    /// This door could not be got to or through.
    GaveUp(OpAddr),
    /// A lock it has no code for, or presses that came to nothing.
    Fail(Why),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Approach,
    Open,
    Through,
    Shut,
}

/// One door on the way: walked up to, opened, walked through, shut.
#[derive(Clone, Copy, Debug)]
struct Hop {
    at: OpAddr,
    stage: Stage,
    /// The spot it is worked from, on this side.
    near: [f32; 2],
    /// The doorway's middle and its normal, pointing to this side.
    mid: [f32; 2],
    out: [f32; 2],
    since: u32,
    /// The walk up's closest to `near` yet: its stage is timed from the
    /// last time it got closer, so a door begun from across the island is
    /// not given up halfway there.
    closest: f32,
    /// A use went out at this tick.
    sent: Option<u32>,
    /// Walking straight: the closest it has come and when, a side-step
    /// round what stands in the way until a tick, and which side is next.
    straight: bool,
    best: Option<(f32, u32)>,
    dodge: Option<(u32, u16)>,
    left: bool,
}

impl Hop {
    fn new(at: OpAddr, x: f32, z: f32, tick: u32) -> Self {
        let (ax, az) = anchor(at.cx, at.cz, at.loc);
        let n = if at.loc == LOC_EDGE_XLO {
            [1.0, 0.0]
        } else {
            [0.0, 1.0]
        };
        let side = if (x - ax) * n[0] + (z - az) * n[1] >= 0.0 {
            1.0
        } else {
            -1.0
        };
        let out = [n[0] * side, n[1] * side];
        Self {
            at,
            stage: Stage::Approach,
            near: [ax + out[0] * HOP_STAND_M, az + out[1] * HOP_STAND_M],
            mid: [ax, az],
            out,
            since: tick,
            closest: f32::MAX,
            sent: None,
            straight: false,
            best: None,
            dodge: None,
            left: false,
        }
    }

    /// Straight at `to`, as a person crosses a room: no closer in
    /// [`DODGE_AFTER_TICKS`] is something in the way, stepped round to one
    /// side for [`DODGE_TICKS`], the other side the next time.
    fn straight_to(&mut self, to: [f32; 2], x: f32, z: f32, tick: u32) -> Intent {
        let yaw = yaw_toward(to[0] - x, to[1] - z);
        if let Some((until, side)) = self.dodge {
            if tick.wrapping_sub(until) > u32::MAX / 2 {
                return Intent::walk(side);
            }
            self.dodge = None;
            self.best = None;
        }
        let left = (to[0] - x).hypot(to[1] - z);
        match self.best {
            Some((best, at)) if left > best - DODGE_PROGRESS_M => {
                if tick.wrapping_sub(at) >= DODGE_AFTER_TICKS {
                    self.left = !self.left;
                    let quarter = 1u16 << 14;
                    let side = if self.left {
                        yaw.wrapping_sub(quarter)
                    } else {
                        yaw.wrapping_add(quarter)
                    };
                    self.dodge = Some((tick.wrapping_add(DODGE_TICKS), side));
                    return Intent::walk(side);
                }
            }
            _ => self.best = Some((left, tick)),
        }
        Intent::walk(yaw)
    }

    /// How far past the doorway, on the far side, (`x`, `z`) stands, and
    /// how far off its middle line.
    fn past(&self, x: f32, z: f32) -> (f32, f32) {
        let (dx, dz) = (x - self.mid[0], z - self.mid[1]);
        (
            -(dx * self.out[0] + dz * self.out[1]),
            (dx * self.out[1] - dz * self.out[0]).abs(),
        )
    }

    /// A point on the doorway's middle line, `m` out on this side (less
    /// than zero: on the far side).
    fn on_line(&self, m: f32) -> [f32; 2] {
        [self.mid[0] + self.out[0] * m, self.mid[1] + self.out[1] * m]
    }

    fn next(&mut self, stage: Stage, tick: u32) {
        self.stage = stage;
        self.since = tick;
        self.closest = f32::MAX;
        self.sent = None;
        self.best = None;
        self.dodge = None;
    }
}

/// The doors in the team's way as a person works them: one at a time,
/// walked up to, opened with `E`, walked through and shut behind. A lock
/// that does not know it is given the team's code first, once.
#[derive(Clone, Copy, Debug, Default)]
pub struct Doors {
    hop: Option<Hop>,
    /// A lock did not know it, here, since this tick: the code goes in...
    knock: Option<(OpAddr, u32)>,
    /// ...and went in at this tick, awaiting the lock's answer.
    entering: Option<(OpAddr, u32)>,
    tries: u8,
}

impl Doors {
    /// A door, a code or a lock's answer in hand.
    pub fn busy(&self) -> bool {
        self.hop.is_some() || self.knock.is_some() || self.entering.is_some()
    }

    /// Through `door` next, from this side.
    pub fn begin(&mut self, door: OpAddr, x: f32, z: f32, tick: u32) {
        self.hop = Some(Hop::new(door, x, z, tick));
    }

    /// [`Doors::begin`] for a door on the way to `goal`: worked from the
    /// side the goal is not on. Where the body stands does not say which
    /// side that is: arriving from behind a base, a body is on the inner
    /// side of its far wall's plane while it is still outside, and a door
    /// worked from there is walked at through the wall.
    pub fn begin_toward(&mut self, door: OpAddr, goal: [f32; 2], tick: u32) {
        let (ax, az) = anchor(door.cx, door.cz, door.loc);
        // The goal reflected through the doorway: a point on its far side.
        self.begin(door, 2.0 * ax - goal[0], 2.0 * az - goal[1], tick);
    }

    /// The lock at `at` refused it: the code goes in there next.
    pub fn knock_at(&mut self, at: OpAddr) {
        self.knock = Some((at, 0));
    }

    /// Drop the door in hand.
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// A step went out at `tick`.
    pub fn sent(&mut self, what: &Through, tick: u32) {
        match *what {
            Through::Use(..) => {
                if let Some(h) = self.hop.as_mut() {
                    h.sent = Some(tick);
                }
            }
            Through::Enter(at, ..) => self.entering = Some((at, tick)),
            _ => {}
        }
    }

    /// A deploy refusal; true when it was this door's. A lock refused the
    /// door pressed (`REFUSE_D_OWNER`): the code goes in first. A code that
    /// went in was not taken; a lock that has locked this body out, or has
    /// no room for it on its list, never will take one.
    pub fn on_refused(&mut self, reason: u8) -> bool {
        let reason = u32::from(reason);
        if self.entering.take().is_some() {
            self.knock = None;
            self.tries = match reason {
                REFUSE_D_LOCKOUT | REFUSE_D_AUTH_FULL => MAX_TRIES,
                _ => self.tries.saturating_add(1),
            };
            return true;
        }
        if reason != REFUSE_D_OWNER {
            return false;
        }
        match self.hop.as_mut().filter(|h| h.sent.is_some()) {
            Some(h) => {
                h.sent = None;
                self.knock = Some((h.at, 0));
                true
            }
            None => false,
        }
    }

    /// A lock let it through: the code entered there was right, and what
    /// it refused is pressed again.
    pub fn on_auth(&mut self, cx: u16, cz: u16, level: u8, loc: u8) {
        if self
            .entering
            .is_some_and(|(at, _)| (at.cx, at.cz, at.level, at.loc) == (cx, cz, level, loc))
        {
            self.entering = None;
            self.knock = None;
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn step(
        &mut self,
        core: &mut ClientCore,
        seed: u64,
        haven: &Haven,
        body: &EntityState,
        hands: &Hands,
        route: &mut Route,
        code: Option<LockCode>,
        tick: u32,
    ) -> Through {
        let (x, z) = (body.qx as f32 * POS_XZ_Q, body.qz as f32 * POS_XZ_Q);
        if self.tries >= MAX_TRIES {
            self.clear();
            return Through::Fail(Why::Refused);
        }
        // The team's code at a lock that did not know it: entered looking
        // at the keypad, then what it refused is pressed again.
        if let Some((at, sent)) = self.entering {
            if tick.wrapping_sub(sent) < VERDICT_TICKS {
                return Through::Go(look_at(seed, haven, core, at));
            }
            self.entering = None;
            self.knock = None;
            self.tries = self.tries.saturating_add(1);
        }
        if let Some((at, since)) = self.knock {
            let Some(code) = code else {
                self.clear();
                return Through::Fail(Why::Locked);
            };
            let since = if since == 0 { tick } else { since };
            self.knock = Some((at, since));
            let intent = Intent {
                look: Look::Point(aim_point(core, seed, haven, at, x, z)),
                ..Intent::IDLE
            };
            if tick.wrapping_sub(since) >= HOLD_TICKS
                && hands.settled()
                && e_picks_by(core, x, z, hands.view().0, at, PRESS_SLACK_M)
            {
                return Through::Enter(at, code, intent);
            }
            if tick.wrapping_sub(since) >= VERDICT_TICKS {
                self.knock = None;
                self.tries = self.tries.saturating_add(1);
            }
            return Through::Go(intent);
        }
        let Some(mut h) = self.hop else {
            return Through::Go(Intent::IDLE);
        };
        if tick.wrapping_sub(h.since) >= HOP_STAGE_TICKS {
            self.hop = None;
            // Through, and only the leaf left open: on regardless.
            return if h.stage == Stage::Shut {
                Through::Passed(h.at)
            } else {
                Through::GaveUp(h.at)
            };
        }
        let door = h.at;
        let aim = |core: &ClientCore| Intent {
            look: Look::Point(aim_point(core, seed, haven, door, x, z)),
            ..Intent::IDLE
        };
        // The press on this door, once the eyes have settled on it and it
        // is what a player's `E` would take from here; one unanswered
        // press counts against the tries.
        let press = |h: &mut Hop, core: &ClientCore, tries: &mut u8| {
            let intent = aim(core);
            match h.sent {
                Some(sent) if tick.wrapping_sub(sent) < VERDICT_TICKS => Through::Go(intent),
                sent => {
                    if sent.is_some() {
                        h.sent = None;
                        *tries = tries.saturating_add(1);
                    }
                    if tick.wrapping_sub(h.since) >= HOLD_TICKS
                        && hands.settled()
                        && e_picks_by(core, x, z, hands.view().0, door, PRESS_SLACK_M)
                    {
                        Through::Use(door, intent)
                    } else {
                        Through::Go(intent)
                    }
                }
            }
        };
        let out = match h.stage {
            Stage::Approach => {
                if (h.near[0] - x).hypot(h.near[1] - z) <= HOP_NEAR_M {
                    h.next(Stage::Open, tick);
                    Through::Go(Intent::IDLE)
                } else {
                    let left = (h.near[0] - x).hypot(h.near[1] - z);
                    if left + HOP_PROGRESS_M < h.closest {
                        h.closest = left;
                        h.since = tick;
                    }
                    let step = if h.straight {
                        Step::Blocked
                    } else {
                        route.to(core, body, h.near, 0.5, false, tick)
                    };
                    match step {
                        step @ Step::Walk { .. } => {
                            Through::Go(step.walk().unwrap_or(Intent::IDLE))
                        }
                        // Far, or a wall or a shut door between: walking
                        // straight would walk into it for the stage's whole
                        // time (the side of a door across its own leaf, or
                        // an airlock's inner door from outside the base).
                        Step::Blocked
                            if left > HOP_STRAIGHT_M || walled_between(core, [x, z], h.near) =>
                        {
                            self.hop = None;
                            return Through::GaveUp(door);
                        }
                        // The route's metre grid has no room by a door
                        // for what stands round it: near, the rest is
                        // walked straight.
                        Step::Blocked => {
                            h.straight = true;
                            Through::Go(h.straight_to(h.near, x, z, tick))
                        }
                        _ => Through::Go(h.straight_to(h.near, x, z, tick)),
                    }
                }
            }
            Stage::Open => match door_open(core, door) {
                // Open, or gone: the doorway is the way.
                Some(true) | None => {
                    h.next(Stage::Through, tick);
                    Through::Go(Intent::IDLE)
                }
                Some(false) => press(&mut h, core, &mut self.tries),
            },
            // Straight, as a person walks a doorway: onto its middle line
            // just short of it, then through.
            Stage::Through => {
                let (past, off) = h.past(x, z);
                if past >= HOP_PAST_M && off <= 1.0 {
                    h.next(Stage::Shut, tick);
                    Through::Go(Intent::IDLE)
                } else if door_open(core, door) == Some(false) {
                    // Shut on it: open it again.
                    h.next(Stage::Open, tick);
                    Through::Go(Intent::IDLE)
                } else {
                    let to = if past < 0.0 && off > HOP_LINE_M {
                        h.on_line(HOP_LINE_OUT_M)
                    } else {
                        h.on_line(-2.0 * HOP_PAST_M)
                    };
                    Through::Go(h.straight_to(to, x, z, tick))
                }
            }
            Stage::Shut => match (door_open(core, door), h.sent) {
                // Shut, or pressed and it did not swing: on regardless.
                (Some(false) | None, _) => {
                    self.hop = None;
                    return Through::Passed(door);
                }
                (Some(true), Some(sent)) if tick.wrapping_sub(sent) >= VERDICT_TICKS => {
                    self.hop = None;
                    return Through::Passed(door);
                }
                (Some(true), _) => press(&mut h, core, &mut self.tries),
            },
        };
        self.hop = Some(h);
        out
    }
}

/// The doors on its floor within `within` of `around`, not among `skip`:
/// the nearest to (`x`, `z`).
pub fn nearest_door(
    core: &ClientCore,
    around: [f32; 2],
    within: f32,
    level: u8,
    skip: &[Option<OpAddr>],
    x: f32,
    z: f32,
) -> Option<OpAddr> {
    let have = core.deploy_defs_have.min(core.deploy_defs.def_count);
    let mut best: Option<(f32, OpAddr)> = None;
    for d in core.deploys.entries() {
        if u16::from(d.row) >= have
            || !matches!(
                core.deploy_defs.defs[usize::from(d.row)].arch,
                ARCH_DOOR | ARCH_GARAGE_DOOR
            )
            || !matches!(d.loc, LOC_EDGE_XLO | LOC_EDGE_ZLO)
            || d.level != level
        {
            continue;
        }
        let at = OpAddr {
            cx: d.cx,
            cz: d.cz,
            level: d.level,
            loc: d.loc,
        };
        let (ax, az) = anchor(d.cx, d.cz, d.loc);
        if skip.contains(&Some(at)) || (ax - around[0]).hypot(az - around[1]) > within {
            continue;
        }
        let dist = (ax - x).hypot(az - z);
        if best.is_none_or(|b| dist < b.0) {
            best = Some((dist, at));
        }
    }
    best.map(|b| b.1)
}

/// A wall, or a doorway with its door shut, stands across the ground-floor
/// line from `a` to `b`: one with a teammate a few metres off, but on the
/// other side of it.
pub fn walled_between(core: &ClientCore, a: [f32; 2], b: [f32; 2]) -> bool {
    let have = core.piece_defs_have;
    core.pieces.entries().iter().any(|r| {
        if r.level != 0 || u16::from(r.row) >= have {
            return false;
        }
        // The piece's line on the ground: an edge along z at x0 (XLO) or
        // along x at z0 (ZLO), or a diagonal across its cell (an airlock's
        // long side).
        let (x0, z0) = (
            f32::from(r.cx) * BUILD_CELL_M,
            f32::from(r.cz) * BUILD_CELL_M,
        );
        let s = BUILD_CELL_M;
        let (p, q) = match r.loc {
            LOC_EDGE_XLO => ([x0, z0], [x0, z0 + s]),
            LOC_EDGE_ZLO => ([x0, z0], [x0 + s, z0]),
            LOC_DIAG_A => ([x0, z0], [x0 + s, z0 + s]),
            LOC_DIAG_B => ([x0 + s, z0], [x0, z0 + s]),
            _ => return false,
        };
        if !crosses(a, b, p, q) {
            return false;
        }
        let shape = core.piece_defs.pieces[usize::from(r.row)].shape;
        if matches!(shape, SHAPE_DOORWAY | SHAPE_FRAME) {
            let addr = OpAddr {
                cx: r.cx,
                cz: r.cz,
                level: r.level,
                loc: r.loc,
            };
            door_open(core, addr) == Some(false)
        } else {
            true
        }
    })
}

/// The segments `a`–`b` and `p`–`q` cross (touching counts).
fn crosses(a: [f32; 2], b: [f32; 2], p: [f32; 2], q: [f32; 2]) -> bool {
    let side = |o: [f32; 2], u: [f32; 2], v: [f32; 2]| {
        (u[0] - o[0]) * (v[1] - o[1]) - (u[1] - o[1]) * (v[0] - o[0])
    };
    let (d1, d2) = (side(p, q, a), side(p, q, b));
    let (d3, d4) = (side(a, b, p), side(a, b, q));
    if d1 * d2 > 0.0 || d3 * d4 > 0.0 {
        return false;
    }
    // Collinear: they cross only where their spans overlap.
    if d1 == 0.0 && d2 == 0.0 {
        let span = |i: usize| a[i].min(b[i]) <= p[i].max(q[i]) && p[i].min(q[i]) <= a[i].max(b[i]);
        return span(0) && span(1);
    }
    true
}

/// Walled in on a built floor: it stands on a foundation or floor, and a
/// wall, a door or a deployable is within [`ENCLOSED_M`] every way it
/// looks (eight bearings, chest high), as a room looks from inside.
pub fn enclosed(core: &mut ClientCore, haven: &Haven, body: &EntityState) -> bool {
    let (x, y, z) = (
        body.qx as f32 * POS_XZ_Q,
        body.qy as f32 * POS_Y_Q,
        body.qz as f32 * POS_XZ_Q,
    );
    let (cx, cz) = (build_cell_of(x), build_cell_of(z));
    if cx < 0 || cz < 0 || core.pieces.cols().plate(cx as u16, cz as u16).is_none() {
        return false;
    }
    let from = [x, y + 1.2, z];
    (0..8u16).all(|k| {
        let (fx, fz) = yaw_dir(k << 13);
        let to = [x + fx * ENCLOSED_M, y + 1.2, z + fz * ENCLOSED_M];
        clear_line(core, haven, from, to, ENCLOSED_M + 0.1) != Sight::Clear
    })
}

/// Out of a teammate's base before a walk into the island, and through a
/// door a teammate went through: [`Doors`] with the doors already passed.
#[derive(Clone, Copy, Debug, Default)]
pub struct Leave {
    pub doors: Doors,
    passed: [Option<OpAddr>; MAX_DOORS],
    n: u8,
    /// The last door passed, and when: a follow does not turn straight
    /// back through it.
    last: Option<(OpAddr, u32)>,
    checked: Option<u32>,
    held: Option<u32>,
}

impl Leave {
    /// A try that came to nothing a moment ago.
    pub fn held(&self, tick: u32) -> bool {
        self.held
            .is_some_and(|t| tick.wrapping_sub(t) < LEAVE_RETRY_TICKS)
    }

    /// Through the nearest door on its floor within `within`, not passed
    /// already on this way out. False when there is none.
    fn through_nearest(
        &mut self,
        core: &ClientCore,
        body: &EntityState,
        within: f32,
        tick: u32,
    ) -> bool {
        let (x, z) = (body.qx as f32 * POS_XZ_Q, body.qz as f32 * POS_XZ_Q);
        if usize::from(self.n) >= MAX_DOORS {
            return false;
        }
        match nearest_door(core, [x, z], within, 0, &self.passed, x, z) {
            Some(door) => {
                self.doors.begin(door, x, z, tick);
                true
            }
            None => false,
        }
    }

    /// After a teammate there is no way to: through the nearest door
    /// within `within`, unless it is the one just passed. False when there
    /// is none, or a try came to nothing a moment ago.
    pub fn after(&mut self, core: &ClientCore, body: &EntityState, within: f32, tick: u32) -> bool {
        if self.held(tick) {
            return false;
        }
        let (x, z) = (body.qx as f32 * POS_XZ_Q, body.qz as f32 * POS_XZ_Q);
        let skip = [self
            .last
            .filter(|&(_, at)| tick.wrapping_sub(at) < BACK_THROUGH_TICKS)
            .map(|(door, _)| door)];
        match nearest_door(core, [x, z], within, 0, &skip, x, z) {
            Some(door) => {
                self.doors.begin(door, x, z, tick);
                true
            }
            None => false,
        }
    }

    /// The door in hand, if any: its next step. Through it, it is
    /// remembered as passed; one that could not be got through holds
    /// further tries a while.
    #[allow(clippy::too_many_arguments)]
    pub fn going(
        &mut self,
        core: &mut ClientCore,
        seed: u64,
        haven: &Haven,
        body: &EntityState,
        hands: &Hands,
        route: &mut Route,
        code: Option<LockCode>,
        tick: u32,
    ) -> Option<Through> {
        if !self.doors.busy() {
            return None;
        }
        let step = self
            .doors
            .step(core, seed, haven, body, hands, route, code, tick);
        match step {
            Through::Passed(at) => {
                if usize::from(self.n) < MAX_DOORS {
                    self.passed[usize::from(self.n)] = Some(at);
                    self.n += 1;
                }
                self.last = Some((at, tick));
                // Look again at once: still walled in is another door.
                self.checked = None;
                Some(Through::Go(Intent::IDLE))
            }
            Through::GaveUp(_) | Through::Fail(_) => {
                self.doors.clear();
                self.held = Some(tick);
                None
            }
            step => Some(step),
        }
    }

    /// Before a walk: walled in on a teammate's floor (once a second it
    /// looks), out through the nearest door. `None` once outside, or when
    /// there is no door to go by.
    #[allow(clippy::too_many_arguments)]
    pub fn out(
        &mut self,
        core: &mut ClientCore,
        seed: u64,
        haven: &Haven,
        body: &EntityState,
        hands: &Hands,
        route: &mut Route,
        code: Option<LockCode>,
        tick: u32,
    ) -> Option<Through> {
        if let Some(step) = self.going(core, seed, haven, body, hands, route, code, tick) {
            return Some(step);
        }
        if self.held(tick) || self.checked.is_some_and(|t| tick.wrapping_sub(t) < TICK_HZ) {
            return None;
        }
        self.checked = Some(tick);
        if core.team().count == 0 || !enclosed(core, haven, body) {
            // Out: the next time in is a new way.
            self.passed = [None; MAX_DOORS];
            self.n = 0;
            return None;
        }
        if !self.through_nearest(core, body, LEAVE_M, tick) {
            self.held = Some(tick);
            return None;
        }
        self.going(core, seed, haven, body, hands, route, code, tick)
    }
}
