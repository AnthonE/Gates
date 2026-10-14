//! Getting between a base's floors: the stairs.
//!
//! The route's grid (`sim_core::nav`) is one layer at the terrain's height:
//! an upper floor is air to it, a flight of stairs a wall. This is the layer
//! above it. A floor is a height ([`Layer`]); a flight of stairs joins the
//! height at its foot to the height at its head, walked along the line
//! `sim_core::circulation::walk_line` proves for its shape. A trip is a
//! shortest path over the flights' ends ([`plan`]): walks on one floor
//! (the grid, at that floor's height: `Nav::plan_on`), climbs and descents
//! between them. Two ends at one height are taken to be on one floor; a
//! walk that finds otherwise is remembered as failed ([`Fails`]) and the
//! trip is planned again without it.
//!
//! The flights come from the pieces this client was sent (`ClientCore`'s
//! `PieceSet`), near the body and near the goal. A flight with a floor laid
//! over its stairwell (no headroom) is left out. Fixed arrays throughout,
//! sized once: nothing here allocates.

use client_core::core::ClientCore;
use sim_core::build::{column_floor_y, level_y, BUILD_CELL_M};
use sim_core::circulation;
use sim_core::collide;
use sim_core::limits::TICK_HZ;
use sim_core::terrain::{self, Haven};

/// Flights a trip may use: the nearest this many around the body and goal.
pub const MAX_FLIGHTS: usize = 32;
/// Points on the longest walk line (the U flight's four).
pub const MAX_LINE: usize = 4;
/// Build cells either side of the body and the goal searched for flights.
pub const WIN_CELLS: i32 = 12;
/// Two heights within this are one floor: a step, `movement::STEP_UP`.
pub const LAYER_M: f32 = 0.6;
/// Standing within this of the dirt is the ground, where the grid walks.
pub const GROUND_M: f32 = 0.9;
/// Feet within this of a built surface are standing on it.
pub const ON_M: f32 = 0.25;
/// Legs a trip may have: a walk and a flight per floor, eight floors.
pub const MAX_LEGS: usize = 18;
/// Walks or flights that came to nothing, remembered for [`FAIL_TICKS`].
pub const MAX_FAILS: usize = 8;
pub const FAIL_TICKS: u32 = 30 * TICK_HZ;
/// A flight costs this much more than its length: stairs are slower.
const FLIGHT_COST_M: f32 = 3.0;
const NODES: usize = 2 + 2 * MAX_FLIGHTS;

/// A height a body can stand at: the ground (the grid's), or a floor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Layer {
    Ground,
    Deck(f32),
}

impl Layer {
    pub fn same(self, other: Layer) -> bool {
        match (self, other) {
            (Layer::Ground, Layer::Ground) => true,
            (Layer::Deck(a), Layer::Deck(b)) => (a - b).abs() <= LAYER_M,
            _ => false,
        }
    }

    pub fn deck(self) -> Option<f32> {
        match self {
            Layer::Deck(y) => Some(y),
            Layer::Ground => None,
        }
    }
}

/// The layer a point at height `y` stands on: the ground within
/// [`GROUND_M`] of the dirt, a floor above it.
pub fn layer_at(seed: u64, haven: &Haven, x: f32, z: f32, y: f32) -> Layer {
    if y - terrain::ground(seed, haven, x, z) <= GROUND_M {
        Layer::Ground
    } else {
        Layer::Deck(y)
    }
}

/// Where a body with feet at `(x, y, z)` stands: on a floor only when its
/// feet are on a built surface well above the dirt. In the air (a jump, a
/// fall) it is the ground's, which is what the grid has always assumed.
pub fn standing(core: &mut ClientCore, x: f32, y: f32, z: f32) -> Layer {
    core.with_ground(|g| {
        let built = collide::body_ground(g.seed, g.haven, g.cols, x, z, y);
        if (built - y).abs() <= ON_M {
            layer_at(g.seed, g.haven, x, z, y)
        } else {
            Layer::Ground
        }
    })
}

/// One flight of stairs, as walked: its line in world metres, foot first.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Flight {
    pub pts: [[f32; 3]; MAX_LINE],
    pub n: u8,
}

impl Flight {
    pub const EMPTY: Self = Self {
        pts: [[0.0; 3]; MAX_LINE],
        n: 0,
    };

    pub fn foot(&self) -> [f32; 3] {
        self.pts[0]
    }

    pub fn head(&self) -> [f32; 3] {
        self.pts[usize::from(self.n.max(1)) - 1]
    }

    /// Metres along the line.
    pub fn length(&self) -> f32 {
        let n = usize::from(self.n);
        (1..n).map(|i| flat(self.pts[i - 1], self.pts[i])).sum()
    }

    /// The line one way or the other: up from the foot, or down from the
    /// head.
    pub fn walk(&self, up: bool) -> Flight {
        let mut out = *self;
        if !up {
            let n = usize::from(self.n);
            for i in 0..n {
                out.pts[i] = self.pts[n - 1 - i];
            }
        }
        out
    }

    /// The segment the line passes nearest `(x, z)` on (`i` for the one
    /// from point `i` to point `i + 1`), if within `within` metres.
    pub fn near(&self, x: f32, z: f32, within: f32) -> Option<usize> {
        let n = usize::from(self.n);
        (0..n.saturating_sub(1))
            .map(|i| (i, seg_dist(self.pts[i], self.pts[i + 1], x, z)))
            .filter(|(_, d)| *d <= within)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    }
}

fn flat(a: [f32; 3], b: [f32; 3]) -> f32 {
    (a[0] - b[0]).hypot(a[2] - b[2])
}

/// Metres from `(x, z)` to the segment `a`–`b`, on the ground plane.
fn seg_dist(a: [f32; 3], b: [f32; 3], x: f32, z: f32) -> f32 {
    let (dx, dz) = (b[0] - a[0], b[2] - a[2]);
    let len2 = dx * dx + dz * dz;
    let t = if len2 > 0.0 {
        (((x - a[0]) * dx + (z - a[2]) * dz) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (a[0] + dx * t - x).hypot(a[2] + dz * t - z)
}

/// The flights near a trip, rebuilt for each plan.
pub struct Flights {
    pub list: [Flight; MAX_FLIGHTS],
    pub n: usize,
}

impl Default for Flights {
    fn default() -> Self {
        Self {
            list: [Flight::EMPTY; MAX_FLIGHTS],
            n: 0,
        }
    }
}

impl Flights {
    pub fn as_slice(&self) -> &[Flight] {
        &self.list[..self.n]
    }

    /// Read the flights within [`WIN_CELLS`] of either point from the
    /// pieces this client holds: every riser, its line in world metres, and
    /// none whose line runs into a ceiling.
    pub fn scan(&mut self, core: &mut ClientCore, around: [[f32; 2]; 2]) {
        self.n = 0;
        let near = |cx: u16, cz: u16| {
            around.iter().any(|p| {
                let (pcx, pcz) = ((p[0] / BUILD_CELL_M) as i32, (p[1] / BUILD_CELL_M) as i32);
                (i32::from(cx) - pcx).abs() <= WIN_CELLS && (i32::from(cz) - pcz).abs() <= WIN_CELLS
            })
        };
        let mut found = [(0u16, 0u16, 0u8, 0u8, 0u8); MAX_FLIGHTS];
        let mut k = 0;
        let defs = &core.piece_defs;
        for p in core.pieces.entries() {
            if k == MAX_FLIGHTS {
                break;
            }
            let Some(def) = defs.pieces.get(usize::from(p.row)) else {
                continue;
            };
            if circulation::is_riser(def.shape) && near(p.cx, p.cz) {
                found[k] = (p.cx, p.cz, p.level, p.loc, def.shape);
                k += 1;
            }
        }
        let list = &mut self.list;
        let mut n = 0;
        core.with_ground(|g| {
            for &(cx, cz, level, loc, shape) in &found[..k] {
                let plate = g.cols.plate(cx, cz).unwrap_or(0);
                let base = column_floor_y(g.seed, g.haven, cx, cz, plate) + level_y(level);
                let line = circulation::walk_line(shape);
                let mut f = Flight::EMPTY;
                let mut clear = true;
                for (i, &(lx, lz)) in line.iter().take(MAX_LINE).enumerate() {
                    let (x, z) = circulation::to_world(loc, lx, lz);
                    let y = base + circulation::surface(shape, loc, x, z).unwrap_or(0.0);
                    let (wx, wz) = (
                        f32::from(cx) * BUILD_CELL_M + x,
                        f32::from(cz) * BUILD_CELL_M + z,
                    );
                    clear &= !collide::plane_blocked(g.seed, g.haven, g.cols, wx, wz, y);
                    f.pts[i] = [wx, y, wz];
                    f.n += 1;
                }
                if clear {
                    list[n] = f;
                    n += 1;
                }
            }
        });
        self.n = n;
    }
}

/// What a trip does next.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Leg {
    /// Walk on one layer to `to`, done within `stop` metres.
    Walk {
        to: [f32; 2],
        layer: Layer,
        stop: f32,
    },
    /// Walk a flight's line, in the order given.
    Climb(Flight),
}

impl Leg {
    pub const NONE: Self = Leg::Walk {
        to: [0.0, 0.0],
        layer: Layer::Ground,
        stop: 0.0,
    };
}

/// A planned trip.
#[derive(Clone, Copy)]
pub struct Trip {
    pub legs: [Leg; MAX_LEGS],
    pub n: usize,
}

impl Default for Trip {
    fn default() -> Self {
        Self {
            legs: [Leg::NONE; MAX_LEGS],
            n: 0,
        }
    }
}

/// Places a walk or a flight came to nothing, for a while.
#[derive(Clone, Copy, Default)]
pub struct Fails {
    at: [Option<([f32; 2], u32)>; MAX_FAILS],
    next: usize,
}

impl Fails {
    pub fn add(&mut self, p: [f32; 2], tick: u32) {
        self.at[self.next] = Some((p, tick));
        self.next = (self.next + 1) % MAX_FAILS;
    }

    pub fn has(&self, p: [f32; 3], tick: u32) -> bool {
        self.at.iter().flatten().any(|&(q, at)| {
            tick.wrapping_sub(at) < FAIL_TICKS && (q[0] - p[0]).hypot(q[1] - p[2]) < 0.5
        })
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

/// Where a trip ends: a point on a layer, or anywhere on the ground.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Target {
    At([f32; 2], Layer, f32),
    AnyGround,
}

/// The shortest trip from a body at `from` (on `here`) to `to`, over
/// `flights`, or `None` if no chain of them gets there. `layer_of` says
/// which layer a flight's end stands on. A body standing on a flight joins
/// at its nearer end; ends `fails` names are left out. Without `direct`
/// the start and the goal are not joined even on one layer: the walk
/// between them was tried and failed (one floor, two rooms).
#[allow(clippy::too_many_arguments)]
pub fn plan(
    from: [f32; 3],
    here: Layer,
    to: Target,
    flights: &[Flight],
    layer_of: impl Fn([f32; 3]) -> Layer,
    fails: &Fails,
    tick: u32,
    direct: bool,
    out: &mut Trip,
) -> bool {
    let nf = flights.len().min(MAX_FLIGHTS);
    // 0 the start, 1 the goal, then each flight's foot and head.
    let mut layer = [Layer::Ground; NODES];
    let mut pos = [[0.0f32; 3]; NODES];
    let n = 2 + 2 * nf;
    layer[0] = here;
    pos[0] = from;
    let (goal_any, goal_stop) = match to {
        Target::At(p, l, stop) => {
            layer[1] = l;
            pos[1] = [p[0], 0.0, p[1]];
            (false, stop)
        }
        Target::AnyGround => (true, 0.5),
    };
    let mut on_flight = None;
    for (k, f) in flights[..nf].iter().enumerate() {
        pos[2 + 2 * k] = f.foot();
        pos[3 + 2 * k] = f.head();
        layer[2 + 2 * k] = layer_of(f.foot());
        layer[3 + 2 * k] = layer_of(f.head());
        if on_flight.is_none() && f.near(from[0], from[2], 0.6).is_some() {
            let (lo, hi) = (f.foot()[1].min(f.head()[1]), f.foot()[1].max(f.head()[1]));
            if from[1] > lo + 0.3 && from[1] < hi - 0.3 {
                on_flight = Some(k);
            }
        }
    }
    let usable = |i: usize| i < 2 || !fails.has(pos[i], tick);
    // Edge cost from node a to node b, if there is one.
    let cost = |a: usize, b: usize| -> Option<f32> {
        if a == b || !usable(a) || !usable(b) {
            return None;
        }
        // A flight's own two ends.
        if a >= 2 && b >= 2 && (a - 2) / 2 == (b - 2) / 2 {
            let f = &flights[(a - 2) / 2];
            return Some(f.length() + FLIGHT_COST_M);
        }
        // Standing on a flight: only its ends.
        if let Some(k) = on_flight {
            if a == 0 || b == 0 {
                let other = if a == 0 { b } else { a };
                return (other >= 2 && (other - 2) / 2 == k).then(|| flat(pos[0], pos[other]));
            }
        }
        if b == 1 && goal_any {
            return (layer[a] == Layer::Ground).then_some(0.0);
        }
        if a == 1 || (!direct && a == 0 && b == 1) {
            return None;
        }
        layer[a].same(layer[b]).then(|| flat(pos[a], pos[b]))
    };
    // Dijkstra, dense: the graph is at most 66 nodes.
    let mut dist = [f32::INFINITY; NODES];
    let mut prev = [u8::MAX; NODES];
    let mut done = [false; NODES];
    dist[0] = 0.0;
    loop {
        let mut u = None;
        for i in 0..n {
            if !done[i] && dist[i].is_finite() && u.is_none_or(|j: usize| dist[i] < dist[j]) {
                u = Some(i);
            }
        }
        let Some(u) = u else {
            break;
        };
        if u == 1 {
            break;
        }
        done[u] = true;
        for v in 0..n {
            if done[v] {
                continue;
            }
            if let Some(c) = cost(u, v) {
                if dist[u] + c < dist[v] {
                    dist[v] = dist[u] + c;
                    prev[v] = u as u8;
                }
            }
        }
    }
    if !dist[1].is_finite() {
        return false;
    }
    // The node chain, goal back to start.
    let mut chain = [0usize; NODES];
    let mut len = 0;
    let mut at = 1usize;
    while at != 0 && len < NODES {
        chain[len] = at;
        len += 1;
        at = usize::from(prev[at]);
    }
    chain[len] = 0;
    len += 1;
    chain[..len].reverse();
    // Legs: a flight's ends one after the other are a climb; else a walk
    // on the layer the step starts from.
    out.n = 0;
    let push = |leg: Leg, out: &mut Trip| {
        if out.n < MAX_LEGS {
            out.legs[out.n] = leg;
            out.n += 1;
        }
    };
    for w in chain[..len].windows(2) {
        let (a, b) = (w[0], w[1]);
        let same_flight = a >= 2 && b >= 2 && (a - 2) / 2 == (b - 2) / 2;
        if same_flight {
            let f = &flights[(a - 2) / 2];
            push(Leg::Climb(f.walk(b % 2 == 1)), out);
        } else if a == 0 && on_flight.is_some() {
            // From a standing start on a flight: the rest of the line to the
            // end chosen.
            let f = &flights[(b - 2) / 2];
            push(Leg::Climb(rest_of(f, b % 2 == 1, from)), out);
        } else if b == 1 && goal_any {
            // Already on the ground at the last end.
        } else {
            let stop = if b == 1 { goal_stop } else { 0.25 };
            let d = flat(pos[a], pos[b]);
            if b == 1 || d > stop {
                push(
                    Leg::Walk {
                        to: [pos[b][0], pos[b][2]],
                        layer: layer[a],
                        stop,
                    },
                    out,
                );
            }
        }
    }
    true
}

/// The part of a flight's line still ahead of a body standing on it,
/// toward the head when `up`.
fn rest_of(f: &Flight, up: bool, from: [f32; 3]) -> Flight {
    let line = f.walk(up);
    let i = line
        .near(from[0], from[2], f32::INFINITY)
        .map_or(0, |i| i + 1);
    let mut out = Flight::EMPTY;
    for p in &line.pts[i..usize::from(line.n)] {
        out.pts[usize::from(out.n)] = *p;
        out.n += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A straight flight from (x, 0, z) up to (x, 3, z + 2.8).
    fn stairs(x: f32, z: f32, base: f32) -> Flight {
        let mut f = Flight::EMPTY;
        f.pts[0] = [x, base + 0.1, z];
        f.pts[1] = [x, base + 2.9, z + 2.8];
        f.n = 2;
        f
    }

    /// Ground at 0, floors at 3 and 6: anything at or under 0.9 is ground.
    fn layer_of(p: [f32; 3]) -> Layer {
        if p[1] <= GROUND_M {
            Layer::Ground
        } else {
            Layer::Deck(p[1])
        }
    }

    fn kinds(t: &Trip) -> Vec<char> {
        t.legs[..t.n]
            .iter()
            .map(|l| match l {
                Leg::Walk { .. } => 'w',
                Leg::Climb(f) if f.foot()[1] < f.head()[1] => 'u',
                Leg::Climb(_) => 'd',
            })
            .collect()
    }

    #[test]
    fn up_two_floors_and_back_down() {
        let flights = [stairs(10.0, 10.0, 0.0), stairs(20.0, 20.0, 3.0)];
        let fails = Fails::default();
        let mut t = Trip::default();
        let goal = Target::At([25.0, 25.0], Layer::Deck(6.0), 0.5);
        assert!(plan(
            [0.0, 0.0, 0.0],
            Layer::Ground,
            goal,
            &flights,
            layer_of,
            &fails,
            0,
            true,
            &mut t
        ));
        assert_eq!(kinds(&t), ['w', 'u', 'w', 'u', 'w']);
        let Leg::Walk { layer, .. } = t.legs[2] else {
            panic!()
        };
        assert!(
            layer.same(Layer::Deck(3.0)),
            "the middle walk is on the first floor"
        );

        // Down from the top floor to anywhere on the ground.
        assert!(plan(
            [25.0, 6.0, 25.0],
            Layer::Deck(6.0),
            Target::AnyGround,
            &flights,
            layer_of,
            &fails,
            0,
            true,
            &mut t
        ));
        assert_eq!(kinds(&t), ['w', 'd', 'w', 'd']);
        let Leg::Climb(last) = t.legs[3] else {
            panic!()
        };
        assert!(last.head()[1] < 1.0, "it ends at the foot, on the ground");
    }

    #[test]
    fn a_failed_end_is_routed_round_and_nothing_is_no_trip() {
        let flights = [stairs(10.0, 10.0, 0.0), stairs(40.0, 10.0, 0.0)];
        let mut fails = Fails::default();
        let mut t = Trip::default();
        let goal = Target::At([12.0, 14.0], Layer::Deck(3.0), 0.5);
        assert!(plan(
            [0.0, 0.0, 0.0],
            Layer::Ground,
            goal,
            &flights,
            layer_of,
            &fails,
            0,
            true,
            &mut t
        ));
        let Leg::Walk { to, .. } = t.legs[0] else {
            panic!()
        };
        assert_eq!(to, [10.0, 10.0], "the nearer flight first");
        fails.add([10.0, 10.0], 0);
        assert!(plan(
            [0.0, 0.0, 0.0],
            Layer::Ground,
            goal,
            &flights,
            layer_of,
            &fails,
            1,
            true,
            &mut t
        ));
        let Leg::Walk { to, .. } = t.legs[0] else {
            panic!()
        };
        assert_eq!(to, [40.0, 10.0], "the failed one is left out");
        let later = FAIL_TICKS + 5;
        assert!(plan(
            [0.0, 0.0, 0.0],
            Layer::Ground,
            goal,
            &flights,
            layer_of,
            &fails,
            later,
            true,
            &mut t
        ));
        let Leg::Walk { to, .. } = t.legs[0] else {
            panic!()
        };
        assert_eq!(to, [10.0, 10.0], "a failure is forgotten");
        let up = Target::At([5.0, 5.0], Layer::Deck(9.0), 0.5);
        assert!(!plan(
            [0.0, 0.0, 0.0],
            Layer::Ground,
            up,
            &flights,
            layer_of,
            &fails,
            0,
            true,
            &mut t
        ));
    }

    #[test]
    fn standing_on_a_flight_finishes_it_either_way() {
        let flights = [stairs(10.0, 10.0, 0.0)];
        let fails = Fails::default();
        let mut t = Trip::default();
        let mid = [10.0, 1.5, 11.4];
        assert!(plan(
            mid,
            Layer::Deck(1.5),
            Target::AnyGround,
            &flights,
            layer_of,
            &fails,
            0,
            true,
            &mut t
        ));
        let Leg::Climb(f) = t.legs[0] else { panic!() };
        assert!(f.head()[1] < 1.0, "down to the foot");
        let goal = Target::At([10.0, 14.0], Layer::Deck(3.0), 0.5);
        assert!(plan(
            mid,
            Layer::Deck(1.5),
            goal,
            &flights,
            layer_of,
            &fails,
            0,
            true,
            &mut t
        ));
        let Leg::Climb(f) = t.legs[0] else { panic!() };
        assert!(f.head()[1] > 2.5, "up to the head");
    }
}
