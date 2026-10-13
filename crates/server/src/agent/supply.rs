//! The team's chest, and filling it.
//!
//! A teammate's chest is the box its eyes saw a teammate standing at
//! ([`TeamChest::look`]): in view, in range, a clear line to it, and an
//! awake teammate within [`CHEST_NEAR_M`] of it. Never the island's list of
//! boxes, nor whose a box is: a player learns where the team keeps its
//! things the same way, by being shown.
//!
//! The delivery ([`DeliverJob`]) walks there with the pack. A closed door
//! in the way is worked the way a person works one: the route says there
//! is no way to the box, so the nearest door near the box is walked up to,
//! opened with `E`, walked through and shut behind ([`Hop`]), and the route
//! is asked again. A lock that does not know it takes the code a teammate
//! gave (an external agent's `"code"`, kept as a [`LockCode`]), entered
//! once at its keypad; with no code the delivery stops there. At the box
//! the panel opens, every stack past the belt's [`stash::loadout`] goes in,
//! and the panel shuts. Then out the way it came, each door opened, passed
//! and shut again.
//!
//! The same door walk ([`Doors`]) takes it after a teammate who went
//! through a door and shut it, and out of a teammate's base before a walk
//! into the island ([`Leave`]): walled in on a built floor, it leaves by
//! the nearest door.

use crate::agent::build::{aim_point, door_open, e_picks_by, look_at, PRESS_SLACK_M};
use crate::agent::hands::Hands;
use crate::agent::home::HOLD_TICKS;
use crate::agent::intent::{yaw_toward, Intent, Look};
use crate::agent::lock::LockCode;
use crate::agent::raid::{BOX_SIGHT_M, BOX_STAND_M};
use crate::agent::route::{Route, Step};
use crate::agent::stash::{self, box_open, Ledger, Transfer};
use crate::agent::tracks::{clear_line, eye_m, Sight, Tracks};
use crate::agent::wiki::Book;
use crate::mind::Why;
use client_core::core::ClientCore;
use protocol::EntityState;
use sim_core::bots::OpAddr;
use sim_core::build::{
    anchor, build_cell_of, BUILD_CELL_M, LOC_EDGE_XLO, LOC_EDGE_ZLO, SHAPE_DOORWAY, SHAPE_FRAME,
};
use sim_core::deploy::{
    box_key, cell_center, ARCH_BOX, ARCH_DOOR, ARCH_GARAGE_DOOR, REFUSE_D_AUTH_FULL,
    REFUSE_D_LOCKOUT, REFUSE_D_OWNER,
};
use sim_core::limits::{BOX_SLOTS, TICK_HZ};
use sim_core::movement::{POS_XZ_Q, POS_Y_Q};
use sim_core::terrain::Haven;
use sim_core::yaw_dir;

/// A teammate standing this near a box is at it: the team's chest.
pub const CHEST_NEAR_M: f32 = 3.0;
/// The eyes look for it this often.
pub const CHEST_LOOK_TICKS: u32 = TICK_HZ / 2;
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
/// A delivery that failed is not offered again for this long.
pub const DELIVER_RETRY_TICKS: u32 = 60 * TICK_HZ;
/// An answer to a press comes within this long.
const VERDICT_TICKS: u32 = 3 * TICK_HZ;

/// Does a box stand at this address, per the mirror?
fn is_box(core: &ClientCore, at: OpAddr) -> bool {
    let have = core.deploy_defs_have.min(core.deploy_defs.def_count);
    core.deploys.entries().iter().any(|d| {
        (d.cx, d.cz, d.level, d.loc) == (at.cx, at.cz, at.level, at.loc)
            && u16::from(d.row) < have
            && core.deploy_defs.defs[usize::from(d.row)].arch == ARCH_BOX
    })
}

/// The box a teammate keeps, as its eyes found it, and what its panel
/// showed the last time it was open.
#[derive(Clone, Copy, Debug, Default)]
pub struct TeamChest {
    at: Option<OpAddr>,
    looked: Option<u32>,
    pub ledger: Ledger,
    /// Times a chest was learnt.
    pub learnt: u64,
}

impl TeamChest {
    pub fn at(&self) -> Option<OpAddr> {
        self.at
    }

    pub fn forget(&mut self) {
        self.at = None;
        self.ledger.clear();
    }

    /// The panel open is the chest's: what it shows is what it holds.
    pub fn on_panel(&mut self, core: &ClientCore) {
        if let Some(at) = self.at {
            self.ledger
                .on_cont(core, box_key(at.cx, at.cz, at.level, at.loc));
        }
    }

    /// Look for a teammate at a box: every [`CHEST_LOOK_TICKS`], each awake
    /// teammate in sight, the box nearest them within [`CHEST_NEAR_M`], in
    /// the view cone and range, and one line of sight to it. Not one of
    /// its own (`own`).
    pub fn look(
        &mut self,
        core: &mut ClientCore,
        haven: &Haven,
        body: &EntityState,
        tracks: &Tracks,
        tick: u32,
        own: impl Fn(u16, u16, u8, u8) -> bool,
    ) {
        if self.at.is_some_and(|at| !is_box(core, at)) {
            self.forget();
        }
        if self
            .looked
            .is_some_and(|t| tick.wrapping_sub(t) < CHEST_LOOK_TICKS)
        {
            return;
        }
        self.looked = Some(tick);
        let team = *core.team();
        // Off the team: a box it was shown is nobody's business now.
        if team.count == 0 {
            self.forget();
            return;
        }
        let feet = [
            body.qx as f32 * POS_XZ_Q,
            body.qy as f32 * POS_Y_Q,
            body.qz as f32 * POS_XZ_Q,
        ];
        let eye = [feet[0], feet[1] + eye_m(body.crouched), feet[2]];
        let (fx, fz) = yaw_dir(body.yaw);
        let have = core.deploy_defs_have.min(core.deploy_defs.def_count);
        let mut best: Option<(f32, OpAddr, f32)> = None;
        for m in team.members() {
            if m.id == core.player_id || m.state != protocol::TEAM_AWAKE {
                continue;
            }
            let Some(mate) = tracks.get(m.id).filter(|t| t.visible) else {
                continue;
            };
            for d in core.deploys.entries() {
                if u16::from(d.row) >= have
                    || core.deploy_defs.defs[usize::from(d.row)].arch != ARCH_BOX
                    || own(d.cx, d.cz, d.level, d.loc)
                {
                    continue;
                }
                let (ax, az) = cell_center(d.cx, d.cz);
                let near = (ax - mate.pos[0]).hypot(az - mate.pos[2]);
                let (dx, dz) = (ax - eye[0], az - eye[2]);
                let dist = dx.hypot(dz);
                let ahead = dx * fx + dz * fz >= dist * std::f32::consts::FRAC_1_SQRT_2;
                if near > CHEST_NEAR_M || dist > BOX_SIGHT_M || (dist >= 0.5 && !ahead) {
                    continue;
                }
                if best.is_none_or(|b| near < b.0) {
                    let at = OpAddr {
                        cx: d.cx,
                        cz: d.cz,
                        level: d.level,
                        loc: d.loc,
                    };
                    best = Some((near, at, mate.pos[1]));
                }
            }
        }
        let Some((_, at, y)) = best else {
            return;
        };
        if self.at == Some(at) {
            return;
        }
        // A line to just short of it, at the height of a box on the floor
        // the teammate stands on.
        let (ax, az) = cell_center(at.cx, at.cz);
        let (dx, dz) = (ax - eye[0], az - eye[2]);
        let d = dx.hypot(dz).max(0.01);
        let target = [ax - dx / d * 0.7, y + 0.5, az - dz / d * 0.7];
        if clear_line(core, haven, eye, target, BOX_SIGHT_M) != Sight::Clear {
            return;
        }
        self.at = Some(at);
        self.ledger.clear();
        self.learnt += 1;
    }
}

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
                    let step = if h.straight {
                        Step::Blocked
                    } else {
                        route.to(core, body, h.near, 0.5, false, tick)
                    };
                    match step {
                        step @ Step::Walk { .. } => {
                            Through::Go(step.walk().unwrap_or(Intent::IDLE))
                        }
                        Step::Blocked if left > HOP_STRAIGHT_M => {
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
fn nearest_door(
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
        if r.level != 0 || !matches!(r.loc, LOC_EDGE_XLO | LOC_EDGE_ZLO) || u16::from(r.row) >= have
        {
            return false;
        }
        // The edge: along z at x0 (XLO), or along x at z0 (ZLO).
        let (x0, z0) = (
            f32::from(r.cx) * BUILD_CELL_M,
            f32::from(r.cz) * BUILD_CELL_M,
        );
        let (across, along, fixed, start) = if r.loc == LOC_EDGE_XLO {
            ((a[0], b[0]), (a[1], b[1]), x0, z0)
        } else {
            ((a[1], b[1]), (a[0], b[0]), z0, x0)
        };
        let span = across.1 - across.0;
        if span.abs() < 1e-4 {
            return false;
        }
        let t = (fixed - across.0) / span;
        let at = along.0 + t * (along.1 - along.0);
        if !(0.0..=1.0).contains(&t) || at < start || at > start + BUILD_CELL_M {
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

/// What the delivery wants next. `explorer.rs` sends the verbs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Deliver {
    /// A door on the way.
    Door(Through),
    /// Open the box's panel.
    Open {
        key: u32,
        intent: Intent,
    },
    /// Pack slot to box slot.
    Put {
        key: u32,
        from: u8,
        to: u8,
        count: u16,
        intent: Intent,
    },
    /// Shut the panel.
    Close(Intent),
    Done,
    Fail(Why),
}

impl Deliver {
    const IDLE: Self = Deliver::Door(Through::Go(Intent::IDLE));
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Leg {
    #[default]
    In,
    Chest,
    Out,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Lid {
    Aiming(u32),
    Opening(u32),
    Moving(u32),
    Open,
}

/// One trip to the team's chest and back out.
#[derive(Clone, Copy, Debug)]
pub struct DeliverJob {
    leg: Leg,
    pub doors: Doors,
    /// The doors passed on the way in, in order...
    passed: [Option<OpAddr>; MAX_DOORS],
    passed_len: u8,
    /// ...and how many of them have been passed again on the way out.
    back: u8,
    /// Doors that would not be got to or through.
    tried: [Option<OpAddr>; MAX_DOORS],
    lid: Lid,
    moved: Option<bool>,
    moving: u16,
    moves: u8,
    chest: Option<OpAddr>,
    tries: u8,
    /// Why the trip failed inside, said once it is back out.
    failed: Option<Why>,
    /// Units put in the chest.
    pub put: u32,
}

impl Default for DeliverJob {
    fn default() -> Self {
        Self {
            leg: Leg::In,
            doors: Doors::default(),
            passed: [None; MAX_DOORS],
            passed_len: 0,
            back: 0,
            tried: [None; MAX_DOORS],
            lid: Lid::Aiming(0),
            moved: None,
            moving: 0,
            moves: 0,
            chest: None,
            tries: 0,
            failed: None,
            put: 0,
        }
    }
}

impl DeliverJob {
    /// The step went out on the wire at `tick`.
    pub fn sent(&mut self, what: &Deliver, tick: u32) {
        match *what {
            Deliver::Door(ref door) => self.doors.sent(door, tick),
            Deliver::Open { .. } => self.lid = Lid::Opening(tick),
            Deliver::Put { count, .. } => {
                self.lid = Lid::Moving(tick);
                self.moved = None;
                self.moving = count;
                self.moves = self.moves.saturating_add(1);
            }
            Deliver::Close(_) => self.leg = Leg::Out,
            _ => {}
        }
    }

    /// A deploy refusal: the door's, or a lock on the box refusing the
    /// move into it (`REFUSE_D_OWNER`), when the code goes in there first.
    pub fn on_refused(&mut self, reason: u8) {
        if self.doors.on_refused(reason) || u32::from(reason) != REFUSE_D_OWNER {
            return;
        }
        if let (Lid::Moving(_), Some(chest)) = (self.lid, self.chest) {
            self.lid = Lid::Open;
            self.moved = None;
            self.doors.knock_at(chest);
        }
    }

    pub fn on_auth(&mut self, cx: u16, cz: u16, level: u8, loc: u8) {
        self.doors.on_auth(cx, cz, level, loc);
    }

    /// The box's panel opened.
    pub fn on_panel(&mut self, handle: u32) {
        if let (Lid::Opening(_), Some(at)) = (self.lid, self.chest) {
            if box_key(at.cx, at.cz, at.level, at.loc) == handle {
                self.lid = Lid::Open;
            }
        }
    }

    /// A move's answer.
    pub fn on_moved(&mut self, refused: bool) {
        if let Lid::Moving(_) = self.lid {
            self.moved = Some(!refused);
            self.lid = Lid::Open;
        }
    }

    /// The trip fails. Inside a door it went through, it walks back out
    /// first, as it came, and says why there.
    fn fail(&mut self, why: Why) -> Deliver {
        if self.leg == Leg::Out || self.back >= self.passed_len {
            return Deliver::Fail(why);
        }
        self.failed = Some(why);
        self.leg = Leg::Out;
        self.doors.clear();
        self.tries = 0;
        Deliver::IDLE
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
        chest: OpAddr,
        code: Option<LockCode>,
        book: &Book,
        tick: u32,
    ) -> Deliver {
        self.chest = Some(chest);
        let (x, z) = (body.qx as f32 * POS_XZ_Q, body.qz as f32 * POS_XZ_Q);
        if self.tries >= MAX_TRIES {
            return self.fail(Why::Refused);
        }
        if self.doors.busy() {
            return match self
                .doors
                .step(core, seed, haven, body, hands, route, code, tick)
            {
                Through::Passed(at) => {
                    if self.leg == Leg::Out {
                        self.back = self.back.saturating_add(1);
                    } else if usize::from(self.passed_len) < MAX_DOORS {
                        self.passed[usize::from(self.passed_len)] = Some(at);
                        self.passed_len += 1;
                    }
                    Deliver::IDLE
                }
                // On the way in another door may do; on the way out it is
                // the only way.
                Through::GaveUp(at) => match self.tried.iter_mut().find(|s| s.is_none()) {
                    Some(slot) if self.leg != Leg::Out => {
                        *slot = Some(at);
                        Deliver::IDLE
                    }
                    _ => self.fail(Why::Stuck),
                },
                Through::Fail(why) => self.fail(why),
                step => Deliver::Door(step),
            };
        }
        match self.leg {
            Leg::In => {
                if !is_box(core, chest) {
                    return self.fail(Why::NotFound);
                }
                let (cx, cz) = cell_center(chest.cx, chest.cz);
                let there = (cx - x).hypot(cz - z) <= BOX_STAND_M;
                let step = if there {
                    Step::Arrived
                } else {
                    route.to(core, body, [cx, cz], BOX_STAND_M - 0.2, false, tick)
                };
                match step {
                    step @ Step::Walk { .. } => {
                        Deliver::Door(Through::Go(step.walk().unwrap_or(Intent::IDLE)))
                    }
                    Step::Wait => {
                        Deliver::Door(Through::Go(Intent::walk(yaw_toward(cx - x, cz - z))))
                    }
                    Step::Arrived => {
                        self.leg = Leg::Chest;
                        self.lid = Lid::Aiming(tick);
                        Deliver::IDLE
                    }
                    // No way to it from here: a door is in the way, the
                    // nearest one near the chest not tried yet.
                    Step::Blocked => {
                        let mut skip = [None; 2 * MAX_DOORS];
                        skip[..MAX_DOORS].copy_from_slice(&self.passed);
                        skip[MAX_DOORS..].copy_from_slice(&self.tried);
                        match nearest_door(core, [cx, cz], HOP_M, chest.level, &skip, x, z) {
                            Some(door) => {
                                self.doors.begin(door, x, z, tick);
                                Deliver::IDLE
                            }
                            None => self.fail(Why::Stuck),
                        }
                    }
                }
            }
            Leg::Chest => self.at_chest(core, seed, haven, body, hands, chest, book, tick),
            Leg::Out => {
                let n = self.passed_len;
                if self.back >= n {
                    return match self.failed {
                        Some(why) => Deliver::Fail(why),
                        None if self.put > 0 => Deliver::Done,
                        None => Deliver::Fail(Why::BoxFull),
                    };
                }
                match self.passed[usize::from(n - 1 - self.back)] {
                    Some(door) => {
                        self.doors.begin(door, x, z, tick);
                        Deliver::IDLE
                    }
                    None => Deliver::Fail(Why::Stuck),
                }
            }
        }
    }

    /// At the chest: the panel opened with `E`, what the pack carries past
    /// the belt put in a stack at a time, then the panel shut.
    #[allow(clippy::too_many_arguments)]
    fn at_chest(
        &mut self,
        core: &mut ClientCore,
        seed: u64,
        haven: &Haven,
        body: &EntityState,
        hands: &Hands,
        chest: OpAddr,
        book: &Book,
        tick: u32,
    ) -> Deliver {
        let (x, z) = (body.qx as f32 * POS_XZ_Q, body.qz as f32 * POS_XZ_Q);
        if !is_box(core, chest) {
            return self.fail(Why::NotFound);
        }
        let key = box_key(chest.cx, chest.cz, chest.level, chest.loc);
        let intent = Intent {
            look: Look::Point(aim_point(core, seed, haven, chest, x, z)),
            ..Intent::IDLE
        };
        match self.lid {
            Lid::Aiming(since) => {
                let (cx, cz) = cell_center(chest.cx, chest.cz);
                if (cx - x).hypot(cz - z) > BOX_STAND_M + 0.5 {
                    // Pushed off it: walk back.
                    self.leg = Leg::In;
                    return Deliver::IDLE;
                }
                if tick.wrapping_sub(since) >= HOLD_TICKS
                    && hands.settled()
                    && e_picks_by(core, x, z, hands.view().0, chest, 0.0)
                {
                    return Deliver::Open { key, intent };
                }
                if tick.wrapping_sub(since) >= VERDICT_TICKS {
                    self.tries = self.tries.saturating_add(1);
                    self.lid = Lid::Aiming(tick);
                }
                Deliver::Door(Through::Go(intent))
            }
            Lid::Opening(sent) => {
                if box_open(core, key) {
                    self.lid = Lid::Open;
                } else if tick.wrapping_sub(sent) >= VERDICT_TICKS {
                    self.tries = self.tries.saturating_add(1);
                    self.lid = Lid::Aiming(tick);
                }
                Deliver::Door(Through::Go(intent))
            }
            Lid::Moving(sent) => {
                if tick.wrapping_sub(sent) >= VERDICT_TICKS {
                    self.lid = Lid::Open;
                    self.moved = Some(false);
                }
                Deliver::Door(Through::Go(intent))
            }
            Lid::Open => {
                let moved = self.moved.take();
                if moved == Some(true) {
                    self.put = self
                        .put
                        .saturating_add(u32::from(std::mem::take(&mut self.moving)));
                }
                if !box_open(core, key) {
                    // The panel shut under it.
                    self.tries = self.tries.saturating_add(1);
                    self.lid = Lid::Aiming(tick);
                    return Deliver::Door(Through::Go(intent));
                }
                if moved == Some(false) || self.moves >= stash::MAX_MOVES {
                    return Deliver::Close(intent);
                }
                let c: &ClientCore = core;
                match stash::plan(
                    c,
                    |item| stash::loadout(c, book, item),
                    &[],
                    &c.cont[..BOX_SLOTS],
                ) {
                    Some(Transfer::Put { from, to, count }) => Deliver::Put {
                        key,
                        from,
                        to,
                        count,
                        intent,
                    },
                    _ => Deliver::Close(intent),
                }
            }
        }
    }
}

/// Does the pack carry something a delivery would put in the chest?
pub fn has_delivery(core: &ClientCore, book: &Book) -> bool {
    matches!(
        stash::plan(
            core,
            |item| stash::loadout(core, book, item),
            &[],
            Ledger::EMPTY.slots()
        ),
        Some(Transfer::Put { .. })
    )
}
