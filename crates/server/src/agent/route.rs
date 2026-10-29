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
//!
//! Between a base's floors the route takes the stairs (`agent::floors`):
//! a goal on another floor than the body's is a trip of walks on one floor
//! and climbs between them, and the grid walks each floor at its own
//! height. Ground to ground is the grid alone, as it always was, until it
//! finds no way toward a goal well off: then the coarse map (`agent::far`)
//! finds the way round, and the grid walks that a waypoint at a time.

use super::far::{Far, Search, FAR_CELL_M, FAR_EXPAND_PER_FRAME, FAR_REACH_M};
use super::floors::{self, Fails, Flight, Flights, Layer, Leg, Target, Trip};
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
/// back and forth without ever stalling. Only a route of legs and guesses
/// is judged so; a path that reaches the goal may go the long way round a
/// wall, and walking it is progress.
pub const GIVE_UP_TICKS: u32 = 10 * TICK_HZ;
/// Closer than the best so far by this much counts as getting closer.
const CLOSER_M: f32 = 1.0;
/// A gap between two calls longer than this starts a fresh progress
/// window: the body was doing something else, not stuck.
const RESUME_GAP_TICKS: u32 = 3;
/// A trip between floors that has not arrived in this long is given up.
pub const TRIP_TICKS: u32 = 90 * TICK_HZ;
/// Or that has been planned again this often.
pub const TRIP_REPLANS: u8 = 6;
/// On a flight: a waypoint is reached within this, the last within the
/// second; no closer in this long is a stall; this far under the line is
/// a fall.
const CLIMB_REACH_M: f32 = 0.15;
const CLIMB_END_M: f32 = 0.25;
const CLIMB_STALL_TICKS: u32 = 2 * TICK_HZ;
const CLIMB_FELL_M: f32 = 1.2;
/// A goal at least this far off that the grid finds no way toward gets a
/// look over the coarse map (`agent::far`). Nearer, no way is a wall or a
/// shut door, which is the skills' to answer (a door to work).
pub const FAR_MIN_M: f32 = 24.0;
/// Searches of the coarse map for one goal, before it is out of reach from
/// here...
pub const FAR_TRIES: u8 = 8;
/// ...and how long a goal not walked toward keeps its count.
pub const FAR_FORGET_TICKS: u32 = 60 * TICK_HZ;
/// A goal that moved less than this is the one the way was found for (a
/// teammate walking on is not a new search a step).
const FAR_SAME_M: f32 = 4.0 * FAR_CELL_M;
/// A body this far from where the coarse map found no way looks again.
const FAR_MOVED_M: f32 = 4.0 * FAR_CELL_M;

/// What the trip in hand is for.
#[derive(Clone, Copy, Debug, PartialEq)]
enum TripFor {
    Goal([f32; 2]),
    Down,
}

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
    /// The first side-step met a wall: the other side (#452).
    StrafeBack,
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
    /// Trips planned between floors, flights walked, and legs of a trip
    /// that failed (each planned again round it).
    pub floor_plans: u64,
    pub flights: u64,
    pub floor_fails: u64,
    /// Searches of the coarse map, the ways they found, and the waypoints
    /// the grid could not get to.
    pub far_searches: u64,
    pub far_found: u64,
    pub far_barred: u64,
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
    /// The path's corner being walked to, the closest to it yet, and when.
    corner: Option<((i32, i32), f32, u32)>,
    unstick: Unstick,
    /// Alternates the side-step, so two stalls in a row try both sides.
    strafe_left: bool,
    /// The leg in hand walks a floor, where a side-step could go over its
    /// edge: the ladder skips it.
    on_deck: bool,
    /// Between floors (`agent::floors`): the flights near the trip, the
    /// trip, which leg of it, and what it is for.
    flights: Box<Flights>,
    trip: Box<Trip>,
    trip_at: usize,
    trip_for: Option<TripFor>,
    trip_target: Target,
    trip_direct: bool,
    trip_started: u32,
    replans: u8,
    /// On a flight: the next point of its line, and the best distance to it
    /// and since when.
    climb_seg: usize,
    climb_best: Option<(f32, u32)>,
    fails: Fails,
    /// A goal no trip could be planned to, its floor, and when: not looked
    /// for again inside [`REPLAN_TICKS`].
    no_trip: Option<([f32; 2], Layer, u32)>,
    /// The long way round (`agent::far`): the goal it is for, the searches
    /// made for it, the last tick it was walked toward, and how far off the
    /// goal was when the last search began.
    far: Box<Far>,
    far_for: Option<[f32; 2]>,
    far_tries: u8,
    far_used: u32,
    far_from_m: f32,
    /// Where the body stood when the search in hand began.
    far_at: [f32; 2],
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
            corner: None,
            unstick: Unstick::Walking,
            strafe_left: false,
            on_deck: false,
            flights: Box::default(),
            trip: Box::default(),
            trip_at: 0,
            trip_for: None,
            trip_target: Target::AnyGround,
            trip_direct: true,
            trip_started: 0,
            replans: 0,
            climb_seg: 0,
            climb_best: None,
            fails: Fails::default(),
            no_trip: None,
            far: Box::default(),
            far_for: None,
            far_tries: 0,
            far_used: 0,
            far_from_m: 0.0,
            far_at: [0.0; 2],
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
        self.corner = None;
        self.unstick = Unstick::Walking;
        self.end_trip();
        self.fails.clear();
        self.far.clear();
        self.far_for = None;
        self.far_tries = 0;
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
        self.to_at(core, body, goal, None, stop_m, hurry, tick)
    }

    /// [`Route::to`] with the goal's height, when it is known (a teammate
    /// seen upstairs): a goal on another floor than the body's is a trip
    /// over the stairs (`agent::floors`). With no height the goal is on
    /// the ground, unless the body stands on a floor that runs out under
    /// it. Ground to ground is [`Route::to`]'s grid alone, unchanged.
    #[allow(clippy::too_many_arguments)]
    pub fn to_at(
        &mut self,
        core: &mut ClientCore,
        body: &EntityState,
        goal: [f32; 2],
        goal_y: Option<f32>,
        stop_m: f32,
        hurry: bool,
        tick: u32,
    ) -> Step {
        let (x, y, z) = feet(body);
        let here = floors::standing(core, x, y, z);
        let want = match goal_y {
            Some(gy) => {
                core.with_ground(|g| floors::layer_at(g.seed, g.haven, goal[0], goal[1], gy))
            }
            None => match here {
                Layer::Deck(d) if deck_under(core, goal, d) => here,
                _ => Layer::Ground,
            },
        };
        // The trip in hand carries on while it is for the same place on the
        // same floor, however the caller says so (a teammate's height is
        // known one frame and only the ground the next).
        if let Some(TripFor::Goal(g)) = self.trip_for {
            let same_goal = (g[0] - goal[0]).hypot(g[1] - goal[1]) <= GOAL_MOVED_M;
            let same_floor = match self.trip_target {
                Target::At(_, layer, _) => layer.same(want),
                Target::AnyGround => false,
            };
            if same_goal && same_floor {
                return self.run_trip(core, body, hurry, tick);
            }
        }
        if self.trip_for.is_some() {
            self.end_trip();
        }
        let mut direct = true;
        match (here, want) {
            (Layer::Ground, Layer::Ground) => {
                return self.ground(core, body, goal, stop_m, hurry, tick);
            }
            (Layer::Deck(d), w) if w.same(here) => {
                let step = self.leg(core, body, goal, stop_m, hurry, tick, Some(d));
                if step != Step::Blocked {
                    return step;
                }
                direct = false;
            }
            _ => {}
        }
        let tried = self.no_trip.is_some_and(|(g, l, at)| {
            (g[0] - goal[0]).hypot(g[1] - goal[1]) <= GOAL_MOVED_M
                && l.same(want)
                && tick.wrapping_sub(at) < REPLAN_TICKS
        });
        let target = Target::At(goal, want, stop_m.max(0.5));
        if !tried && self.start_trip(core, body, here, target, direct, tick) {
            self.trip_for = Some(TripFor::Goal(goal));
            return self.run_trip(core, body, hurry, tick);
        }
        if !tried {
            self.no_trip = Some((goal, want, tick));
        }
        // No stairs for it: what the route always did.
        self.leg(core, body, goal, stop_m, hurry, tick, None)
    }

    /// Get down to the ground by the stairs, from wherever on a base's
    /// floors the body stands. `Arrived` on the ground; `Blocked` when no
    /// flight leads down.
    pub fn down(&mut self, core: &mut ClientCore, body: &EntityState, tick: u32) -> Step {
        if self.trip_for == Some(TripFor::Down) {
            return self.run_trip(core, body, false, tick);
        }
        let (x, y, z) = feet(body);
        let here = floors::standing(core, x, y, z);
        if here == Layer::Ground {
            return Step::Arrived;
        }
        if self.trip_for.is_some() {
            self.end_trip();
        }
        if self.start_trip(core, body, here, Target::AnyGround, true, tick) {
            self.trip_for = Some(TripFor::Down);
            return self.run_trip(core, body, false, tick);
        }
        Step::Blocked
    }

    /// A trip down the stairs is under way: it finishes before anything
    /// else walks, even once the last steps are low enough to count as
    /// the ground.
    pub fn descending(&self) -> bool {
        self.trip_for == Some(TripFor::Down)
    }

    /// The body stands on a base's floor, not the ground.
    pub fn upstairs(&self, core: &mut ClientCore, body: &EntityState) -> bool {
        let (x, y, z) = feet(body);
        floors::standing(core, x, y, z) != Layer::Ground
    }

    /// The trip between floors in hand, if any, is walking a flight.
    pub fn climbing(&self) -> bool {
        self.trip_for.is_some() && matches!(self.trip.legs.get(self.trip_at), Some(Leg::Climb(_)))
    }

    fn end_trip(&mut self) {
        self.trip.n = 0;
        self.trip_at = 0;
        self.trip_for = None;
        self.replans = 0;
        self.climb_seg = 0;
        self.climb_best = None;
    }

    /// Plan a trip from where the body stands; true when there is one.
    fn start_trip(
        &mut self,
        core: &mut ClientCore,
        body: &EntityState,
        here: Layer,
        target: Target,
        direct: bool,
        tick: u32,
    ) -> bool {
        let (x, y, z) = feet(body);
        let far = match target {
            Target::At(p, ..) => p,
            Target::AnyGround => [x, z],
        };
        self.flights.scan(core, [[x, z], far]);
        let (flights, fails, trip) = (&self.flights, &self.fails, &mut self.trip);
        let found = core.with_ground(|g| {
            floors::plan(
                [x, y, z],
                here,
                target,
                flights.as_slice(),
                |p| floors::layer_at(g.seed, g.haven, p[0], p[2], p[1]),
                fails,
                tick,
                direct,
                trip,
            )
        });
        self.stats.floor_plans += 1;
        if !found {
            return false;
        }
        if self.trip_for.is_none() {
            self.trip_started = tick;
            self.replans = 0;
        }
        self.trip_target = target;
        self.trip_direct = direct;
        self.trip_at = 0;
        self.climb_seg = 0;
        self.climb_best = None;
        true
    }

    /// The trip's next step: its legs in order, each planned again round
    /// a leg that fails, until it arrives or runs out of time or tries.
    fn run_trip(
        &mut self,
        core: &mut ClientCore,
        body: &EntityState,
        hurry: bool,
        tick: u32,
    ) -> Step {
        if tick.wrapping_sub(self.trip_started) >= TRIP_TICKS || self.replans > TRIP_REPLANS {
            self.end_trip();
            self.stats.blocked += 1;
            return Step::Blocked;
        }
        loop {
            if self.trip_at >= self.trip.n {
                self.end_trip();
                return Step::Arrived;
            }
            match self.trip.legs[self.trip_at] {
                Leg::Walk { to, layer, stop } => {
                    match self.leg(core, body, to, stop, hurry, tick, layer.deck()) {
                        Step::Arrived => self.trip_at += 1,
                        Step::Blocked => {
                            self.fails.add(to, tick);
                            return self.replan(core, body, hurry, tick);
                        }
                        // Never stand waiting on a floor: the callers would
                        // walk straight, and a floor has edges. The leg's
                        // end is near and on the same floor.
                        Step::Wait => {
                            let (x, _, z) = feet(body);
                            return Step::Walk {
                                yaw: yaw_toward(to[0] - x, to[1] - z),
                                sprint: false,
                                jump: false,
                            };
                        }
                        step => return step,
                    }
                }
                Leg::Climb(f) => match self.climb(&f, body, tick) {
                    Ok(Some(step)) => return step,
                    Ok(None) => {
                        self.stats.flights += 1;
                        self.trip_at += 1;
                        self.climb_seg = 0;
                        self.climb_best = None;
                    }
                    Err(()) => {
                        let foot = f.foot();
                        self.fails.add([foot[0], foot[2]], tick);
                        return self.replan(core, body, hurry, tick);
                    }
                },
            }
        }
    }

    fn replan(
        &mut self,
        core: &mut ClientCore,
        body: &EntityState,
        hurry: bool,
        tick: u32,
    ) -> Step {
        self.stats.floor_fails += 1;
        self.replans += 1;
        self.path.clear();
        self.last = None;
        let (x, y, z) = feet(body);
        let here = floors::standing(core, x, y, z);
        let (target, direct) = (self.trip_target, self.trip_direct);
        if self.replans <= TRIP_REPLANS && self.start_trip(core, body, here, target, direct, tick) {
            return self.run_trip(core, body, hurry, tick);
        }
        self.end_trip();
        self.stats.blocked += 1;
        Step::Blocked
    }

    /// One step along a flight's line, point to point at a walk: `None`
    /// at its end, `Err` on a stall or a fall off it.
    fn climb(&mut self, f: &Flight, body: &EntityState, tick: u32) -> Result<Option<Step>, ()> {
        let (x, y, z) = feet(body);
        let n = usize::from(f.n);
        while self.climb_seg < n {
            let p = f.pts[self.climb_seg];
            let reach = if self.climb_seg + 1 == n {
                CLIMB_END_M
            } else {
                CLIMB_REACH_M
            };
            let d = (p[0] - x).hypot(p[2] - z);
            if d <= reach {
                self.climb_seg += 1;
                self.climb_best = None;
                continue;
            }
            let under = f.pts[self.climb_seg.saturating_sub(1)][1].min(p[1]);
            if y < under - CLIMB_FELL_M {
                return Err(());
            }
            match self.climb_best {
                Some((best, _)) if d < best - 0.05 => self.climb_best = Some((d, tick)),
                Some((_, at)) if tick.wrapping_sub(at) >= CLIMB_STALL_TICKS => return Err(()),
                Some(_) => {}
                None => self.climb_best = Some((d, tick)),
            }
            return Ok(Some(Step::Walk {
                yaw: yaw_toward(p[0] - x, p[2] - z),
                sprint: false,
                jump: false,
            }));
        }
        Ok(None)
    }

    /// Ground to ground: the grid, and the long way round when it finds
    /// no way toward a goal well off. The grid's window ends 48 m out and
    /// a plan spends a few hundred cells, so from the foot of a cliff, or a
    /// ledge on one, toward a goal past it, nothing it can reach is closer
    /// and it says "no way" at once; a walk home offered over and over
    /// failed so in 0 s each time. A goal the coarse map finds no way to
    /// either, or that [`FAR_TRIES`] searches have not got it to, is out of
    /// reach from here.
    fn ground(
        &mut self,
        core: &mut ClientCore,
        body: &EntityState,
        goal: [f32; 2],
        stop_m: f32,
        hurry: bool,
        tick: u32,
    ) -> Step {
        let (x, y, z) = feet(body);
        let same = self
            .far_for
            .is_some_and(|g| (g[0] - goal[0]).hypot(g[1] - goal[1]) <= FAR_SAME_M);
        if !same || tick.wrapping_sub(self.far_used) >= FAR_FORGET_TICKS {
            if self.far_for.is_some() {
                self.far.clear();
            }
            self.far_for = None;
            self.far_tries = 0;
        }
        // No way from where it stood is not no way from here.
        if self.far.state() == Search::NoWay
            && (self.far_at[0] - x).hypot(self.far_at[1] - z) > FAR_MOVED_M
        {
            self.far.done();
        }
        if self.far_for.is_some() {
            self.far_used = tick;
            if let Some(step) = self.detour(core, body, hurry, tick) {
                return step;
            }
        }
        let step = self.leg(core, body, goal, stop_m, hurry, tick, None);
        let left = (goal[0] - x).hypot(goal[1] - z);
        if step != Step::Blocked || left < FAR_MIN_M || self.far_tries >= FAR_TRIES {
            return step;
        }
        self.far.begin([x, y, z], goal);
        self.far_for = Some(goal);
        self.far_tries += 1;
        self.far_used = tick;
        self.far_from_m = left;
        self.far_at = [x, z];
        self.stats.far_searches += 1;
        self.detour(core, body, hurry, tick)
            .unwrap_or(Step::Blocked)
    }

    /// The long way round's next step: the search while it runs, then the
    /// grid to each waypoint in turn. `None` once the way is walked (the
    /// grid takes the goal from there); `Blocked` when there is no way.
    fn detour(
        &mut self,
        core: &mut ClientCore,
        body: &EntityState,
        hurry: bool,
        tick: u32,
    ) -> Option<Step> {
        if self.far.state() == Search::Running {
            let far = &mut self.far;
            core.with_ground(|g| far.run(g, FAR_EXPAND_PER_FRAME));
            if self.far.state() == Search::Found {
                self.stats.far_found += 1;
            }
        }
        match self.far.state() {
            Search::Running => return Some(Step::Wait),
            Search::NoWay => return Some(Step::Blocked),
            Search::Idle => return None,
            Search::Found => {}
        }
        let (x, y, z) = feet(body);
        while let Some(p) = self.far.point() {
            if (p[0] - x).hypot(p[1] - z) > FAR_REACH_M {
                break;
            }
            self.far.advance();
        }
        let Some(p) = self.far.point() else {
            // Walked. A way that got it well on toward the goal gives the
            // next search its try back.
            self.far.done();
            let goal = self.far.target();
            if (goal[0] - x).hypot(goal[1] - z) <= self.far_from_m * 0.6 {
                self.far_tries = self.far_tries.saturating_sub(1);
            }
            return None;
        };
        match self.leg(core, body, p, FAR_REACH_M, hurry, tick, None) {
            Step::Arrived => {
                self.far.advance();
                Some(Step::Wait)
            }
            // Something the coarse map does not show is in the way there
            // (a base, a rock, a crag): round it, while tries last. Near
            // the goal, what is in the way is the skill's to answer (the
            // walls of the base it is going into, and their doors).
            Step::Blocked => {
                self.stats.far_barred += 1;
                self.far.bar(p);
                let goal = self.far.target();
                if self.far_tries >= FAR_TRIES || (goal[0] - x).hypot(goal[1] - z) < FAR_MIN_M {
                    self.far.done();
                    return Some(Step::Blocked);
                }
                self.far.begin([x, y, z], goal);
                self.far_tries += 1;
                self.far_from_m = (goal[0] - x).hypot(goal[1] - z);
                self.far_at = [x, z];
                self.stats.far_searches += 1;
                Some(Step::Wait)
            }
            step => Some(step),
        }
    }

    /// One step toward `goal` on one layer: the grid at the terrain's
    /// height, or at a floor's (`deck`).
    #[allow(clippy::too_many_arguments)]
    fn leg(
        &mut self,
        core: &mut ClientCore,
        body: &EntityState,
        goal: [f32; 2],
        stop_m: f32,
        hurry: bool,
        tick: u32,
        deck: Option<f32>,
    ) -> Step {
        self.on_deck = deck.is_some();
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
        let whole = matches!(self.last, Some(Plan::Found | Plan::Direct));
        match self.best {
            Some((best, at)) if !resumed && !whole && left + CLOSER_M > best => {
                if tick.wrapping_sub(at) >= GIVE_UP_TICKS {
                    self.best = None;
                    self.stats.blocked += 1;
                    self.path.clear();
                    self.last = None;
                    return Step::Blocked;
                }
            }
            // Closer, a fresh start, or on a whole path: the clock runs
            // from here.
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
            let plan = core.with_ground(|g| match deck {
                Some(d) => nav.plan_on(g, x, y, z, goal[0], goal[1], stop_cm, d, path),
                None => nav.plan(g, x, y, z, goal[0], goal[1], stop_cm, path),
            });
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
        // A whole path is judged by the corner it walks to: walking one,
        // the body gets closer to it. A corner it has come no closer to in
        // `GIVE_UP_TICKS` is one it cannot get to (the capsule held by
        // what the grid let through), however busy the legs: a side-step
        // moves the body enough to start the stall ladder over every time,
        // and it never got to the end of it.
        let corner = (tx, tz);
        let near = ((tx - body.qx) as f32 * POS_XZ_Q).hypot((tz - body.qz) as f32 * POS_XZ_Q);
        match self.corner {
            Some((c, best, at)) if c == corner && !resumed && near + CLOSER_M > best => {
                if tick.wrapping_sub(at) >= GIVE_UP_TICKS {
                    self.corner = None;
                    self.stats.blocked += 1;
                    self.path.clear();
                    self.last = None;
                    return Step::Blocked;
                }
            }
            _ => self.corner = Some((corner, near, tick)),
        }
        let mut yaw = yaw_toward(
            (tx - body.qx) as f32 * POS_XZ_Q,
            (tz - body.qz) as f32 * POS_XZ_Q,
        );
        if matches!(self.unstick, Unstick::Strafe | Unstick::StrafeBack) {
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
            // No side-step on a floor: it could go over the edge.
            Unstick::Jump if self.on_deck => Unstick::Replan,
            Unstick::Jump => {
                self.strafe_left = !self.strafe_left;
                Unstick::Strafe
            }
            Unstick::Strafe => {
                self.strafe_left = !self.strafe_left;
                Unstick::StrafeBack
            }
            Unstick::StrafeBack => Unstick::Replan,
            Unstick::Replan => {
                self.unstick = Unstick::Walking;
                return true;
            }
        };
        false
    }
}

/// The body's feet, metres.
fn feet(body: &EntityState) -> (f32, f32, f32) {
    (
        body.qx as f32 * POS_XZ_Q,
        body.qy as f32 * POS_Y_Q,
        body.qz as f32 * POS_XZ_Q,
    )
}

/// A floor at about height `d` runs under `goal`: the same floor, as far
/// as a walk on it can tell.
fn deck_under(core: &mut ClientCore, goal: [f32; 2], d: f32) -> bool {
    core.with_ground(|g| {
        let s = sim_core::collide::body_ground(g.seed, g.haven, g.cols, goal[0], goal[1], d);
        (s - d).abs() <= floors::LAYER_M
    })
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
    /// does, one rung a second: jump, side-step each way, plan again, give up. The
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
        // Walk, jump, a side-step each way, replan: then given up.
        assert!((5 * STALL_TICKS..6 * STALL_TICKS).contains(&at), "{at}");
        // One plan for the route, one for the replan rung.
        assert_eq!(route.stats.plans, 2, "{:?}", route.stats);
    }

    /// The report's dead end (seed 20260731, 2026-10-19): a body on a
    /// cliff face whose goal lies up and past it. The grid alone says no
    /// way at once, which failed a walk home in 0 s every time it was
    /// offered; the coarse map finds the way round, and the body walks it
    /// there under the sim's own movement.
    #[test]
    fn a_goal_past_a_cliff_face_is_walked_to_the_long_way_round() {
        use sim_core::input::InputFrame;
        let seed = 20260731;
        let haven = terrain::haven(seed);
        let mut core = ClientCore::new(seed, 1, 0);
        let mut body = sim_core::movement::Body::at(seed, &haven, 2810.5, 2594.5);
        let goal = [3088.0, 2709.0];
        let state = |b: &sim_core::movement::Body| EntityState {
            id: 1,
            qx: b.qx,
            qy: b.qy,
            qz: b.qz,
            grounded: b.grounded,
            ..EntityState::default()
        };
        // The grid alone: no way.
        let mut grid = Route::new();
        grid.begin_tick();
        let me = state(&body);
        let (x, y, z) = feet(&me);
        let alone = grid.leg(&mut core, &me, goal, 2.0, false, 0, None);
        assert_eq!(alone, Step::Blocked, "the dead end, from ({x}, {y}, {z})");

        let mut route = Route::new();
        let mut arrived = None;
        for tick in 0..240 * TICK_HZ {
            route.begin_tick();
            let me = state(&body);
            let frame = match route.to(&mut core, &me, goal, 2.0, false, tick) {
                Step::Walk { yaw, jump, .. } => InputFrame {
                    yaw,
                    move_z: 127,
                    buttons: if jump { BTN_JUMP } else { 0 },
                    ..InputFrame::default()
                },
                Step::Wait => InputFrame::default(),
                Step::Arrived => {
                    arrived = Some(tick);
                    break;
                }
                Step::Blocked => {
                    panic!("blocked at {tick} at {:?}: {:?}", feet(&me), route.stats)
                }
            };
            core.with_ground(|g| {
                sim_core::movement::step(g.seed, g.haven, g.cols, g.occ, &mut body, &frame)
            });
        }
        let me = state(&body);
        assert!(
            arrived.is_some(),
            "not there: at {:?}, {:?}",
            feet(&me),
            route.stats
        );
        assert!(route.stats.far_found >= 1, "{:?}", route.stats);
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
