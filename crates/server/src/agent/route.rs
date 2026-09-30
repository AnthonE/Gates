//! Getting somewhere on foot: the animals' grid A* (`sim_core::nav`) run
//! over the ground this client predicts on, so a route agrees with the
//! capsule about what is solid. Pieces and closed doors are walls, because
//! the collision index `ClientCore::with_ground` lends says so; getting
//! through a door of one's own is a skill's business, not the route's.
//!
//! Bounded per frame like the sim's own searches: at most one plan a frame,
//! at most one a second unless the path in hand is no good any more, and a
//! plan expands at most `NAV_MAX_EXPAND` cells. The scratch behind it is
//! allocated once, with the body.
//!
//! A body that stops making progress on a route does what a person does:
//! jumps, then side-steps, then thinks again, and only then gives up.

use super::intent::{yaw_toward, Intent};
use client_core::core::ClientCore;
use protocol::EntityState;
use sim_core::gather::REACH_M;
use sim_core::input::{BTN_JUMP, BTN_SPRINT};
use sim_core::limits::{NAV_EXPAND_PER_TICK, NAV_MAX_EXPAND, TICK_HZ};
use sim_core::movement::{POS_XZ_Q, POS_Y_Q, WADE_GROUND_MAX};
use sim_core::nav::{self, Nav, NavPath, Plan};
use sim_core::terrain::{self, Haven, ISLAND_SIZE};

/// Plans a frame may run. One: a plan is the frame path's dearest step.
pub const PLANS_PER_FRAME: u32 = 1;
/// Cells one frame's planning may expand: one plan's worth.
pub const EXPAND_PER_FRAME: u32 = PLANS_PER_FRAME * NAV_MAX_EXPAND;
const _: () = assert!(
    EXPAND_PER_FRAME <= NAV_EXPAND_PER_TICK,
    "a frame's plans must fit the budget `Nav::begin_tick` refills, or one defers"
);
/// A path still in hand is replanned at most this often (a second).
pub const REPLAN_TICKS: u32 = TICK_HZ;
/// A goal that moved this far since its plan makes the path stale.
pub const GOAL_MOVED_M: f32 = 2.0;
/// Progress is judged over windows this long.
pub const STALL_TICKS: u32 = TICK_HZ;
/// Less progress than this in a window is a stall.
pub const STALL_M: f32 = 0.5;
/// A goal the body has got no closer to in this long is given up, however
/// busy the legs are: a route that ends each leg where the last began walks
/// back and forth without ever stalling.
pub const GIVE_UP_TICKS: u32 = 10 * TICK_HZ;
/// Closer than the best so far by this much counts as getting closer.
const CLOSER_M: f32 = 1.0;
/// A gap between two calls longer than this starts a fresh progress
/// window: the body was doing something else, not stuck.
const RESUME_GAP_TICKS: u32 = 3;

/// What the legs should do this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Walk along this world bearing; sprint if asked to hurry; jump when
    /// the route is fighting a lip it cannot step.
    Walk { yaw: u16, sprint: bool, jump: bool },
    /// Within the stop distance of the goal.
    Arrived,
    /// No way there from here, as far as this body can tell.
    Blocked,
    /// No path yet and none may be planned this frame.
    Wait,
}

impl Step {
    /// The walk as an intent: legs on the bearing, eyes along it.
    pub fn walk(self) -> Option<Intent> {
        let Step::Walk { yaw, sprint, jump } = self else {
            return None;
        };
        let mut buttons = 0;
        if sprint {
            buttons |= BTN_SPRINT;
        }
        if jump {
            buttons |= BTN_JUMP;
        }
        Some(Intent {
            buttons,
            ..Intent::walk(yaw)
        })
    }
}

/// How a stalled route is being worked loose.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Unstick {
    #[default]
    Walking,
    Jump,
    Strafe,
    Replan,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct RouteStats {
    /// Plans run, straight-line answers included.
    pub plans: u64,
    /// Plans the per-tick budget deferred.
    pub deferred: u64,
    /// Goals given up as unreachable.
    pub blocked: u64,
    /// Stalls worked on (each escalation counts once).
    pub stalls: u64,
    /// Cells expanded, all plans.
    pub expanded: u64,
}

pub struct Route {
    nav: Box<Nav>,
    path: NavPath,
    /// The goal the path was planned for, metres, and its stop distance.
    goal: Option<[f32; 2]>,
    stop_m: f32,
    last: Option<Plan>,
    planned_at: Option<u32>,
    plans_this_frame: u32,
    /// Where the current progress window opened.
    mark: Option<(u32, i32, i32)>,
    /// The closest this body has come to the goal, and when.
    best: Option<(f32, u32)>,
    /// The path in hand has had a corner to walk to since it was planned.
    walked: bool,
    called_at: Option<u32>,
    unstick: Unstick,
    /// Alternates the side-step, so two stalls in a row try both sides.
    strafe_left: bool,
    pub stats: RouteStats,
}

impl Default for Route {
    fn default() -> Self {
        Self::new()
    }
}

impl Route {
    pub fn new() -> Self {
        Self {
            nav: Box::new(Nav::new()),
            path: NavPath::default(),
            goal: None,
            stop_m: 0.0,
            last: None,
            planned_at: None,
            plans_this_frame: 0,
            mark: None,
            best: None,
            walked: false,
            called_at: None,
            unstick: Unstick::Walking,
            strafe_left: false,
            stats: RouteStats::default(),
        }
    }

    /// Drop the route in hand (a new life, a new session); the scratch is
    /// kept.
    pub fn reset(&mut self) {
        self.path.clear();
        self.goal = None;
        self.last = None;
        self.planned_at = None;
        self.mark = None;
        self.best = None;
        self.called_at = None;
        self.unstick = Unstick::Walking;
    }

    /// Once per frame, before any skill asks for a step.
    pub fn begin_tick(&mut self) {
        self.nav.begin_tick();
        self.plans_this_frame = 0;
    }

    /// The goal the route is walking to, if any.
    pub fn goal(&self) -> Option<[f32; 2]> {
        self.goal
    }

    /// One step toward `goal` (metres), done within `stop_m` of it.
    pub fn to(
        &mut self,
        core: &mut ClientCore,
        body: &EntityState,
        goal: [f32; 2],
        stop_m: f32,
        hurry: bool,
        tick: u32,
    ) -> Step {
        let (x, z) = (body.qx as f32 * POS_XZ_Q, body.qz as f32 * POS_XZ_Q);
        let stop_m = stop_m.max(0.5);
        let moved = self.goal.is_none_or(|g| {
            (g[0] - goal[0]).hypot(g[1] - goal[1]) > GOAL_MOVED_M || self.stop_m != stop_m
        });
        if moved {
            self.goal = Some(goal);
            self.stop_m = stop_m;
            self.path.clear();
            self.last = None;
            self.mark = None;
            self.best = None;
            self.unstick = Unstick::Walking;
        }
        let resumed = self
            .called_at
            .is_none_or(|at| tick.wrapping_sub(at) > RESUME_GAP_TICKS);
        self.called_at = Some(tick);
        let left = (goal[0] - x).hypot(goal[1] - z);
        if left <= stop_m {
            self.mark = None;
            self.best = None;
            self.unstick = Unstick::Walking;
            return Step::Arrived;
        }
        match self.best {
            Some((best, at)) if !resumed && left + CLOSER_M > best => {
                if tick.wrapping_sub(at) >= GIVE_UP_TICKS {
                    self.best = None;
                    self.stats.blocked += 1;
                    self.path.clear();
                    self.last = None;
                    return Step::Blocked;
                }
            }
            // Closer, or a fresh start: the clock runs from here.
            _ => self.best = Some((left, tick)),
        }
        if self.stalled(body, tick, resumed) {
            self.stats.blocked += 1;
            self.path.clear();
            self.last = None;
            return Step::Blocked;
        }

        let mut next = nav::waypoint(&mut self.path, body.qx, body.qz);
        self.walked |= next.is_some();
        // A path that ran out short of the goal was a leg of it (the goal
        // lay past the window) or all there was; a stall at Replan wants a
        // fresh look. Either way plan, if this frame and the pace allow. A
        // plan that gave nothing to walk is not retried inside the second.
        let spent = next.is_none();
        let paced = self
            .planned_at
            .is_none_or(|at| tick.wrapping_sub(at) >= REPLAN_TICKS);
        // No way there, and too soon to look again.
        if spent && !paced && matches!(self.last, Some(Plan::Unreachable | Plan::NoWay)) {
            self.stats.blocked += 1;
            return Step::Blocked;
        }
        let invalid = spent && (self.last.is_none() || self.walked);
        let stale = (spent || self.unstick == Unstick::Replan) && paced;
        if (invalid || stale) && self.plans_this_frame < PLANS_PER_FRAME {
            self.plans_this_frame += 1;
            let y = body.qy as f32 * POS_Y_Q;
            let stop_cm = (stop_m * 100.0) as u16;
            let (nav, path) = (&mut self.nav, &mut self.path);
            let before = nav.expanded;
            let plan = core.with_ground(|g| nav.plan(g, x, y, z, goal[0], goal[1], stop_cm, path));
            self.stats.plans += 1;
            self.stats.expanded += nav.expanded - before;
            match plan {
                Plan::Deferred => self.stats.deferred += 1,
                Plan::NoWay => {
                    self.stats.blocked += 1;
                    self.path.clear();
                    self.last = Some(Plan::NoWay);
                    self.planned_at = Some(tick);
                    return Step::Blocked;
                }
                plan => {
                    self.last = Some(plan);
                    self.planned_at = Some(tick);
                    self.walked = false;
                }
            }
            next = nav::waypoint(&mut self.path, body.qx, body.qz);
            self.walked |= next.is_some();
        }
        let Some((tx, tz)) = next else {
            return Step::Wait;
        };
        let mut yaw = yaw_toward(
            (tx - body.qx) as f32 * POS_XZ_Q,
            (tz - body.qz) as f32 * POS_XZ_Q,
        );
        if self.unstick == Unstick::Strafe {
            let quarter = 1u16 << 14;
            yaw = if self.strafe_left {
                yaw.wrapping_sub(quarter)
            } else {
                yaw.wrapping_add(quarter)
            };
        }
        Step::Walk {
            yaw,
            sprint: hurry,
            jump: self.unstick == Unstick::Jump,
        }
    }

    /// Judge the progress window; escalate a stall. True when the ladder
    /// is spent and the goal should be given up.
    fn stalled(&mut self, body: &EntityState, tick: u32, resumed: bool) -> bool {
        let Some((at, qx, qz)) = self.mark.filter(|_| !resumed) else {
            self.mark = Some((tick, body.qx, body.qz));
            return false;
        };
        if tick.wrapping_sub(at) < STALL_TICKS {
            return false;
        }
        self.mark = Some((tick, body.qx, body.qz));
        let moved = ((body.qx - qx) as f32 * POS_XZ_Q).hypot((body.qz - qz) as f32 * POS_XZ_Q);
        if moved >= STALL_M {
            self.unstick = Unstick::Walking;
            return false;
        }
        self.stats.stalls += 1;
        self.unstick = match self.unstick {
            Unstick::Walking => Unstick::Jump,
            Unstick::Jump => {
                self.strafe_left = !self.strafe_left;
                Unstick::Strafe
            }
            Unstick::Strafe => Unstick::Replan,
            Unstick::Replan => {
                self.unstick = Unstick::Walking;
                return true;
            }
        };
        false
    }
}

/// Whether a step along `yaw` wades into deeper water than the body stands
/// in: the shore a walk turns back from.
pub fn into_deeper_water(core: &mut ClientCore, body: &EntityState, yaw: u16) -> bool {
    let x = body.qx as f32 * POS_XZ_Q;
    let z = body.qz as f32 * POS_XZ_Q;
    let (dx, dz) = sim_core::yaw_dir(yaw);
    let (seed, island) = core.island();
    let here = terrain::ground(seed, island.haven, x, z);
    let ahead = terrain::ground(seed, island.haven, x + dx * REACH_M, z + dz * REACH_M);
    ahead <= WADE_GROUND_MAX && ahead < here
}

/// Island cells on a side of the explored map.
pub const FRONTIER_SIDE: usize = 128;
/// Metres a side of one explored-map cell: the resource sight range, so a
/// cell walked through is a cell looked over.
pub const FRONTIER_CELL_M: f32 = ISLAND_SIZE / FRONTIER_SIDE as f32;
/// Rings searched for an unvisited cell, one pick at a time.
pub const FRONTIER_RINGS: i32 = 8;
const FRONTIER_WORDS: usize = FRONTIER_SIDE * FRONTIER_SIDE / 64;

/// Where this body has been, coarsely, and where it has not: what a player
/// keeps in their head of a map they are learning.
pub struct Frontier {
    visited: [u64; FRONTIER_WORDS],
    /// The cell being walked to, as (cx, cz).
    pub target: Option<(i32, i32)>,
}

impl Default for Frontier {
    fn default() -> Self {
        Self::new()
    }
}

impl Frontier {
    pub const fn new() -> Self {
        Self {
            visited: [0; FRONTIER_WORDS],
            target: None,
        }
    }

    pub fn clear(&mut self) {
        self.visited = [0; FRONTIER_WORDS];
        self.target = None;
    }

    pub fn cell_of(x: f32, z: f32) -> (i32, i32) {
        (
            (x / FRONTIER_CELL_M).floor() as i32,
            (z / FRONTIER_CELL_M).floor() as i32,
        )
    }

    pub fn centre(cell: (i32, i32)) -> [f32; 2] {
        [
            (cell.0 as f32 + 0.5) * FRONTIER_CELL_M,
            (cell.1 as f32 + 0.5) * FRONTIER_CELL_M,
        ]
    }

    fn index(cell: (i32, i32)) -> Option<usize> {
        let side = FRONTIER_SIDE as i32;
        ((0..side).contains(&cell.0) && (0..side).contains(&cell.1))
            .then_some(cell.1 as usize * FRONTIER_SIDE + cell.0 as usize)
    }

    pub fn visited(&self, cell: (i32, i32)) -> bool {
        Self::index(cell).is_none_or(|i| self.visited[i / 64] & 1 << (i % 64) != 0)
    }

    pub fn visit(&mut self, cell: (i32, i32)) {
        if let Some(i) = Self::index(cell) {
            self.visited[i / 64] |= 1 << (i % 64);
        }
        if self.target == Some(cell) {
            self.target = None;
        }
    }

    /// The nearest unvisited cell on dry land, ring by ring, preferring
    /// cells ahead of `heading` so a walk keeps its direction instead of
    /// zig-zagging between equal choices. `None` when every cell within
    /// the rings is visited or sea.
    pub fn pick(
        &mut self,
        seed: u64,
        haven: &Haven,
        x: f32,
        z: f32,
        heading: u16,
    ) -> Option<(i32, i32)> {
        let here = Self::cell_of(x, z);
        let (hx, hz) = sim_core::yaw_dir(heading);
        for r in 1..=FRONTIER_RINGS {
            let mut best: Option<(f32, (i32, i32))> = None;
            for dz in -r..=r {
                for dx in -r..=r {
                    if dx.abs() != r && dz.abs() != r {
                        continue;
                    }
                    let cell = (here.0 + dx, here.1 + dz);
                    if self.visited(cell) {
                        continue;
                    }
                    let [cx, cz] = Self::centre(cell);
                    if terrain::ground(seed, haven, cx, cz) <= terrain::BEACH_MAX_H {
                        continue;
                    }
                    let (ox, oz) = (cx - x, cz - z);
                    let d = ox.hypot(oz);
                    // Behind costs up to two cells more than ahead.
                    let ahead = (ox * hx + oz * hz) / d.max(1.0);
                    let score = d + (1.0 - ahead) * FRONTIER_CELL_M;
                    if best.is_none_or(|b| score < b.0) {
                        best = Some((score, cell));
                    }
                }
            }
            if let Some((_, cell)) = best {
                self.target = Some(cell);
                return Some(cell);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_core::movement::{quant_xz, quant_y};

    /// A body that makes no progress works itself loose the way a person
    /// does, one rung a second: jump, side-step, plan again, give up. The
    /// route plans at most once a frame, and a path in hand is not
    /// replanned inside the second.
    #[test]
    fn a_stalled_route_jumps_then_side_steps_then_replans_then_gives_up() {
        let seed = 20260731;
        let haven = terrain::haven(seed);
        let mut core = ClientCore::new(seed, 1, 0);
        // Open, dry ground with a clear line 20 m east.
        let mut spot = None;
        'find: for i in 0..400 {
            let (x, z) = (1600.0 + i as f32 * 4.0, 1600.0);
            if terrain::ground(seed, &haven, x, z) <= terrain::BEACH_MAX_H {
                continue;
            }
            let y = terrain::ground(seed, &haven, x, z);
            let clear = core.with_ground(|g| Nav::new().clear_line(g, x, y, z, x + 20.0, z));
            if clear {
                spot = Some((x, y, z));
                break 'find;
            }
        }
        let (x, y, z) = spot.expect("open ground");
        let body = EntityState {
            id: 1,
            qx: quant_xz(x),
            qy: quant_y(y),
            qz: quant_xz(z),
            grounded: true,
            ..EntityState::default()
        };
        let goal = [x + 20.0, z];
        let mut route = Route::new();
        let mut rungs = [0u32; 4];
        let mut blocked_at = None;
        for tick in 0..6 * TICK_HZ {
            route.begin_tick();
            let plans = route.stats.plans;
            let step = route.to(&mut core, &body, goal, 1.0, false, tick);
            assert!(route.stats.plans - plans <= u64::from(PLANS_PER_FRAME));
            match step {
                Step::Walk { yaw, jump, .. } => {
                    let east = 1u16 << 14;
                    let side = yaw.wrapping_sub(east) as i16;
                    let rung = if jump {
                        1
                    } else if side.unsigned_abs() >= 1 << 13 {
                        2
                    } else {
                        0
                    };
                    rungs[rung] += 1;
                    // Jumping comes only after a second without progress,
                    // side-stepping only after the jump.
                    assert!(tick >= rung as u32 * STALL_TICKS, "rung {rung} at {tick}");
                }
                Step::Blocked => {
                    blocked_at = Some(tick);
                    break;
                }
                other => panic!("{other:?} at {tick}"),
            }
        }
        assert!(rungs[0] > 0 && rungs[1] > 0 && rungs[2] > 0, "{rungs:?}");
        let at = blocked_at.expect("a route that never moves is given up");
        assert!((4 * STALL_TICKS..5 * STALL_TICKS).contains(&at), "{at}");
        // One plan for the route, one for the replan rung.
        assert_eq!(route.stats.plans, 2, "{:?}", route.stats);
    }

    #[test]
    fn the_frontier_keeps_ahead_and_skips_what_it_has_walked() {
        let seed = 20260731;
        let haven = terrain::haven(seed);
        // A dry spot with dry land all round it.
        let mut at = None;
        'find: for i in 40..90 {
            for j in 40..90 {
                let c = Frontier::centre((i, j));
                let dry = (-2..=2).all(|dz| {
                    (-2..=2).all(|dx| {
                        let d = Frontier::centre((i + dx, j + dz));
                        terrain::ground(seed, &haven, d[0], d[1]) > terrain::BEACH_MAX_H
                    })
                });
                if dry {
                    at = Some(c);
                    break 'find;
                }
            }
        }
        let [x, z] = at.expect("an inland spot");
        let here = Frontier::cell_of(x, z);
        let mut f = Frontier::new();
        f.visit(here);
        // Heading +Z: the pick lies ahead.
        let cell = f.pick(seed, &haven, x, z, 0).unwrap();
        assert_eq!(cell, (here.0, here.1 + 1));
        // Walked there already: the next pick is a different cell.
        f.visit(cell);
        let next = f.pick(seed, &haven, x, z, 0).unwrap();
        assert_ne!(next, cell);
        assert!(!f.visited(next));
        // Heading -X: the pick turns that way.
        let west = f.pick(seed, &haven, x, z, 3 << 14).unwrap();
        assert_eq!(west.0, here.0 - 1);
    }
}
