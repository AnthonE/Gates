//! Animal pathing — the reference game's navigator, on a grid we never bake
//! (`reference/ANIMALS.md` §2 A).
//!
//! **What the reference does.** Its NPCs walk a navmesh the server bakes at
//! boot. Building blocks *carve* that mesh (`ai.nav_carve_*`), so an animal
//! goes around a base instead of into its wall. A brain asks its navigator
//! for a destination at a speed (`BaseNavigator.SetDestination(pos, speed)`),
//! the agent walks the path's corners, and a path that stops short of its
//! goal (Unity's `PathPartial`) is how "I can't get to you" reaches the
//! brain. A* on a node graph covers the places the mesh does not reach.
//!
//! **What this does.** The same contract with no bake. A* runs on a 1 m grid
//! whose cells are probed lazily from the predicates `movement::step` itself
//! uses: terrain height and the cliff ratio, `collide::blocked` for walls,
//! and the occupant, deploy and plane-flank vetoes. So a piece is carved
//! the tick it is placed and uncarved the tick it breaks, and a path can
//! never disagree with the capsule about what is solid, because they ask
//! the same functions. `ANIMALS.md` §9.1 argued this module out of
//! existence because the heightfield is analytic, which is true of terrain
//! and false of a base; steering straight at a player on the other side of
//! a wall walks the animal into the wall forever.
//!
//! **Bounded like everything else in the tick.** A search lives inside a
//! `NAV_WIN`-cell window, expands at most `NAV_MAX_EXPAND` cells, and draws
//! on a per-tick expansion budget (`NAV_EXPAND_PER_TICK`) that stands in for
//! the reference's `ai.framebudgetms`. Theirs is a millisecond budget; ours
//! counts cells, because a clock is not a determinism input. Out of budget,
//! a request is *deferred*: the animal keeps its old path and asks again on
//! its next think. A straight line is tried first and is the common answer
//! on open ground, so most requests expand nothing at all.
//!
//! **Scratch, not state.** `Nav` holds no fact about the world between two
//! searches: every cell is generation-stamped and re-probed the first time
//! a search touches it, and the budget resets every tick. So it is not
//! hashed and not saved, for `SlotCache`'s reason. What *is* state is the
//! path a search hands back, which lives on the mob and is hashed there.

use crate::collide::{self, ColIndex};
use crate::fmath::floor_i32;
use crate::limits::{NAV_EXPAND_PER_TICK, NAV_HEAP_CAP, NAV_MAX_CORNERS, NAV_MAX_EXPAND, NAV_WIN};
use crate::movement::{BORDER_MARGIN, POS_XZ_Q, STEP_UP};
use crate::occupy::Occupants;
use crate::terrain::{self, Haven, CLIFF_SLOPE_RATIO, ISLAND_SIZE};
use crate::yaw_lut::yaw_dir;

/// Nav cell edge, metres. A capsule is 0.8 m across, so 1 m cells resolve
/// a gap between two trunks and a doorway without ever pretending a wall is
/// thinner than it is.
pub const NAV_CELL_M: f32 = 1.0;

/// Cells per window: the heap packs a window index into 14 bits.
const WIN_CELLS: usize = NAV_WIN * NAV_WIN;
const _: () = assert!(
    WIN_CELLS <= 1 << 14,
    "a window index must fit the heap key's 14 bits"
);
const _: () = assert!(
    NAV_WIN >= 16,
    "a window this small cannot route around a base"
);

/// Step costs, tenths of a metre. Diagonal is √2 rounded.
const COST_STRAIGHT: u32 = 10;
const COST_DIAG: u32 = 14;
/// Extra per cell on the beach band: sand is walkable and not preferred,
/// which keeps a route off the coast the way the leash keeps a roam off it.
const COST_BEACH: u32 = 12;
/// Extra per cell of water, on top of the beach's. The capsule wades any
/// depth at half speed, so water is not a wall — an animal that ends up in
/// it must be able to route out — but a route through a bay has to be a
/// lot shorter than the way round before it wins.
const COST_WATER: u32 = 40;
/// Extra per decimetre climbed. Mild on purpose: a hill is a detour only
/// when the way round is short.
const COST_CLIMB_PER_DM: u32 = 1;

/// The largest drop one edge may take, metres. The capsule falls happily,
/// but a route off a ledge it cannot climb back up strands the animal.
const MAX_DROP_M: f32 = 2.0;

/// A corner counts as reached inside this many centimetres.
const CORNER_REACH_CM: i64 = 100;
/// The tightest a goal may be stopped at: one step of a sprint is 18 cm,
/// so anything under a few steps is a target the body orbits.
const MIN_STOP_CM: i64 = 50;

/// Cells a blocked goal may be nudged to find standing room (a player on a
/// boulder, a roam point on a trunk).
const GOAL_NUDGE: i32 = 2;

/// The most edges one path may spend being straightened. The pull is a
/// nicety and must not be the worst case of a search.
const PULL_EDGE_BUDGET: u32 = 2_048;

/// A request that would get less than this out of what is left of the
/// tick's budget waits for the next tick rather than coming back partial.
const MIN_EXPAND: u32 = 64;

const F_PROBED: u8 = 1;
const F_BLOCKED: u8 = 2;
const F_HARD: u8 = 4;
const F_BEACH: u8 = 8;
const F_CLOSED: u8 = 16;
const F_SEEN: u8 = 32;
const F_WATER: u8 = 64;

/// `parent` of the start cell.
const PARENT_NONE: u8 = 8;

/// The eight neighbours, straight first. A diagonal may be taken only when
/// both straight cells beside it are open, so a route never squeezes
/// between a wall's end and a tree.
const DIRS: [(i32, i32); 8] = [
    (1, 0),
    (-1, 0),
    (0, 1),
    (0, -1),
    (1, 1),
    (1, -1),
    (-1, 1),
    (-1, -1),
];

/// A path, as the mob carries it: turn points in global nav-cell
/// coordinates and the exact goal. Sim state (it decides where the body
/// goes), hashed with the mob.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NavPath {
    pub cx: [u16; NAV_MAX_CORNERS],
    pub cz: [u16; NAV_MAX_CORNERS],
    /// Corners in use; zero is "no path".
    pub len: u8,
    /// The corner being walked to.
    pub next: u8,
    /// The exact goal, position quanta — the last corner is steered here
    /// rather than to its cell centre.
    pub goal_qx: i32,
    pub goal_qz: i32,
    /// How close to the goal counts as there, centimetres.
    pub stop_cm: u16,
    /// The search stopped short of the goal (no way through, out of window
    /// or out of budget) and this path ends at the closest point it found.
    pub partial: bool,
    /// The last corner was reached. Stays set until the next plan, which is
    /// how a state learns it arrived.
    pub arrived: bool,
}

impl NavPath {
    #[inline]
    pub fn active(&self) -> bool {
        self.next < self.len
    }

    pub fn clear(&mut self) {
        self.len = 0;
        self.next = 0;
        self.partial = false;
        self.arrived = false;
    }
}

/// What a plan came back as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plan {
    /// A straight line was clear; the path is the goal.
    Direct,
    /// A* reached the goal.
    Found,
    /// The goal is past the window: the path walks toward it, and the
    /// brain plans again from where it ends.
    Toward,
    /// A* ran out of expansions (or the route out of corners) first; the
    /// path ends at the closest cell reached. Maybe reachable.
    Partial,
    /// A* ran out of *frontier* first: nothing inside the window reaches
    /// the goal. The path still ends at the closest cell reached — the
    /// reference's `PathPartial`, which is how "I can't get to you" is
    /// told apart from "not yet".
    Unreachable,
    /// Nowhere to go from here (boxed in, or already as close as it gets).
    NoWay,
    /// Out of this tick's budget. Nothing was written.
    Deferred,
}

impl Plan {
    /// A path was written and it leads somewhere.
    #[inline]
    pub fn moving(self) -> bool {
        matches!(
            self,
            Plan::Direct | Plan::Found | Plan::Toward | Plan::Partial | Plan::Unreachable
        )
    }
}

/// Everything a probe asks. The same bundle `movement::step` is handed, so
/// a cell is walkable exactly when the capsule could stand in it.
pub struct Ground<'a, 'o> {
    pub seed: u64,
    pub haven: &'a Haven,
    pub cols: &'a ColIndex,
    pub occ: &'a mut Occupants<'o>,
}

/// Search scratch, shared by the whole roster (one search runs at a time).
pub struct Nav {
    stamp: Box<[u16; WIN_CELLS]>,
    flags: Box<[u8; WIN_CELLS]>,
    y: Box<[f32; WIN_CELLS]>,
    g: Box<[u32; WIN_CELLS]>,
    parent: Box<[u8; WIN_CELLS]>,
    heap: Box<[u64; NAV_HEAP_CAP]>,
    heap_len: usize,
    chain: Box<[u16; WIN_CELLS]>,
    gen: u16,
    ox: i32,
    oz: i32,
    budget: u32,
    pull_left: u32,
    /// Cells expanded since construction — the profile reads it.
    pub expanded: u64,
    /// Searches run since construction (straight-line answers included).
    pub plans: u64,
}

impl Default for Nav {
    fn default() -> Self {
        Self::new()
    }
}

impl Nav {
    pub fn new() -> Self {
        Self {
            stamp: crate::boxed_array(0),
            flags: crate::boxed_array(0),
            y: crate::boxed_array(0.0),
            g: crate::boxed_array(0),
            parent: crate::boxed_array(0),
            heap: crate::boxed_array(0),
            heap_len: 0,
            chain: crate::boxed_array(0),
            gen: 0,
            ox: 0,
            oz: 0,
            budget: NAV_EXPAND_PER_TICK,
            pull_left: 0,
            expanded: 0,
            plans: 0,
        }
    }

    /// Refill the expansion budget. Called once per tick, before the roster
    /// thinks.
    pub fn begin_tick(&mut self) {
        self.budget = NAV_EXPAND_PER_TICK;
    }

    /// Plan from a body standing at `(x, y, z)` to `(gx, gz)`, writing the
    /// result into `out` unless the plan is `Deferred`.
    #[allow(clippy::too_many_arguments)]
    pub fn plan(
        &mut self,
        gr: &mut Ground,
        x: f32,
        y: f32,
        z: f32,
        gx: f32,
        gz: f32,
        stop_cm: u16,
        out: &mut NavPath,
    ) -> Plan {
        self.plans += 1;
        let (scx, scz) = (cell_of(x), cell_of(z));
        let (mut gcx, mut gcz) = (cell_of(gx), cell_of(gz));
        self.begin(scx, scz, gcx, gcz);
        // A goal past the window is walked toward: clamp it onto the window
        // along the line, and the path comes back partial so the brain asks
        // again from where it ends up.
        let mut clipped = false;
        if self.local(gcx, gcz).is_none() {
            let (cx, cz) = self.clip(scx, scz, gcx, gcz);
            gcx = cx;
            gcz = cz;
            clipped = true;
        }
        let Some(si) = self.local(scx, scz) else {
            return Plan::NoWay;
        };
        // The start is wherever the body stands, whatever a probe would
        // make of its cell — being inside a veto must never be absorbing.
        self.stamp[si] = self.gen;
        self.flags[si] = F_PROBED | F_SEEN;
        self.y[si] = y;
        self.g[si] = 0;
        self.parent[si] = PARENT_NONE;

        let asked = (gcx, gcz);
        let (gcx, gcz) = self.nudge(gr, gcx, gcz);
        let Some(gi) = self.local(gcx, gcz) else {
            return Plan::NoWay;
        };
        // The exact goal, unless it was moved: a clipped or nudged goal is
        // steered to its cell's centre, never back into the trunk it was
        // nudged out of.
        let goal_q = if clipped || (gcx, gcz) != asked {
            (movement_q(centre(gcx)), movement_q(centre(gcz)))
        } else {
            (movement_q(gx), movement_q(gz))
        };
        let reached_as = if clipped { Plan::Toward } else { Plan::Direct };
        if si == gi {
            write_path(out, &[(gcx, gcz)], goal_q, stop_cm, clipped);
            return reached_as;
        }

        // The straight line first: open ground is most of the island, and a
        // walkable line is a path with nothing to search.
        self.pull_left = PULL_EDGE_BUDGET;
        if self.line(gr, scx, scz, gcx, gcz) {
            write_path(out, &[(gcx, gcz)], goal_q, stop_cm, clipped);
            return reached_as;
        }

        let cap = NAV_MAX_EXPAND.min(self.budget);
        if cap < MIN_EXPAND {
            return Plan::Deferred;
        }
        let (end, reached, exhausted) = self.astar(gr, si, gi, gcx, gcz, cap);
        if end == si {
            return Plan::NoWay;
        }
        let n = self.reconstruct(end, si);
        let mut corners = [(0i32, 0i32); NAV_MAX_CORNERS];
        self.pull_left = PULL_EDGE_BUDGET;
        let (k, truncated) = self.pull(gr, n, &mut corners);
        // A route that stops short steers to its own last cell, not the
        // goal it did not reach.
        let whole = reached && !truncated;
        let q = if whole {
            goal_q
        } else {
            let (cx, cz) = corners[k - 1];
            (movement_q(centre(cx)), movement_q(centre(cz)))
        };
        write_path(out, &corners[..k], q, stop_cm, !whole || clipped);
        match (whole, clipped, exhausted) {
            (true, false, _) => Plan::Found,
            (true, true, _) => Plan::Toward,
            (false, _, true) => Plan::Unreachable,
            (false, _, false) => Plan::Partial,
        }
    }

    /// Whether a body at `(x, y, z)` can walk straight to `(gx, gz)` — the
    /// line check a plan tries first, offered on its own so a state can ask
    /// "is it clear?" without paying for a path.
    #[allow(clippy::too_many_arguments)]
    pub fn clear_line(
        &mut self,
        gr: &mut Ground,
        x: f32,
        y: f32,
        z: f32,
        gx: f32,
        gz: f32,
    ) -> bool {
        let (scx, scz) = (cell_of(x), cell_of(z));
        let (gcx, gcz) = (cell_of(gx), cell_of(gz));
        self.begin(scx, scz, gcx, gcz);
        let (Some(si), Some(_)) = (self.local(scx, scz), self.local(gcx, gcz)) else {
            return false;
        };
        self.stamp[si] = self.gen;
        self.flags[si] = F_PROBED | F_SEEN;
        self.y[si] = y;
        self.pull_left = PULL_EDGE_BUDGET;
        self.line(gr, scx, scz, gcx, gcz)
    }

    /// Whether an animal would choose to stand at `(x, z)` — open, and dry
    /// — the probe a state uses to vet a roam or flee point before it
    /// spends a search on one. Water is routable and never a destination.
    pub fn standable(&mut self, gr: &mut Ground, x: f32, z: f32) -> bool {
        let (f, _) = probe(gr, cell_of(x), cell_of(z));
        f & (F_BLOCKED | F_WATER) == 0
    }

    /// Open a new window around start and goal. The midpoint when both fit
    /// with room to go round something; otherwise the start, and the goal
    /// is clipped onto it.
    fn begin(&mut self, scx: i32, scz: i32, gcx: i32, gcz: i32) {
        self.gen = self.gen.wrapping_add(1);
        if self.gen == 0 {
            // Wrapped: every stamp is ambiguous now, so none may survive.
            self.stamp.fill(0);
            self.gen = 1;
        }
        self.heap_len = 0;
        let half = NAV_WIN as i32 / 2;
        let margin = NAV_WIN as i32 / 8;
        let fits = (gcx - scx).abs() <= NAV_WIN as i32 - 2 * margin
            && (gcz - scz).abs() <= NAV_WIN as i32 - 2 * margin;
        let (mx, mz) = if fits {
            ((scx + gcx) / 2, (scz + gcz) / 2)
        } else {
            (scx, scz)
        };
        self.ox = mx - half;
        self.oz = mz - half;
    }

    #[inline]
    fn local(&self, cx: i32, cz: i32) -> Option<usize> {
        let lx = cx - self.ox;
        let lz = cz - self.oz;
        if lx < 0 || lz < 0 || lx >= NAV_WIN as i32 || lz >= NAV_WIN as i32 {
            return None;
        }
        Some(lz as usize * NAV_WIN + lx as usize)
    }

    /// The last in-window cell on the line from the start toward a goal
    /// outside the window, one cell in from the edge.
    fn clip(&self, scx: i32, scz: i32, gcx: i32, gcz: i32) -> (i32, i32) {
        let (dx, dz) = ((gcx - scx) as f32, (gcz - scz) as f32);
        let lo_x = (self.ox + 1) as f32;
        let hi_x = (self.ox + NAV_WIN as i32 - 2) as f32;
        let lo_z = (self.oz + 1) as f32;
        let hi_z = (self.oz + NAV_WIN as i32 - 2) as f32;
        let (sx, sz) = (scx as f32, scz as f32);
        let mut t = 1.0f32;
        if sx + dx * t > hi_x && dx > 0.0 {
            t = t.min((hi_x - sx) / dx);
        }
        if sx + dx * t < lo_x && dx < 0.0 {
            t = t.min((lo_x - sx) / dx);
        }
        if sz + dz * t > hi_z && dz > 0.0 {
            t = t.min((hi_z - sz) / dz);
        }
        if sz + dz * t < lo_z && dz < 0.0 {
            t = t.min((lo_z - sz) / dz);
        }
        let t = t.max(0.0);
        (
            floor_i32(sx + dx * t + 0.5).clamp(self.ox + 1, self.ox + NAV_WIN as i32 - 2),
            floor_i32(sz + dz * t + 0.5).clamp(self.oz + 1, self.oz + NAV_WIN as i32 - 2),
        )
    }

    /// The cell's flags and standing height, probed once per search.
    fn cell(&mut self, gr: &mut Ground, i: usize, cx: i32, cz: i32) -> u8 {
        if self.stamp[i] != self.gen {
            self.stamp[i] = self.gen;
            self.flags[i] = 0;
        }
        if self.flags[i] & F_PROBED == 0 {
            let (f, y) = probe(gr, cx, cz);
            self.flags[i] |= f;
            self.y[i] = y;
        }
        self.flags[i]
    }

    /// Move a goal that lands on something solid to the nearest open cell
    /// within `GOAL_NUDGE`, nearest ring first. Left alone if none is.
    fn nudge(&mut self, gr: &mut Ground, gcx: i32, gcz: i32) -> (i32, i32) {
        let Some(i) = self.local(gcx, gcz) else {
            return (gcx, gcz);
        };
        if self.cell(gr, i, gcx, gcz) & F_BLOCKED == 0 {
            return (gcx, gcz);
        }
        for r in 1..=GOAL_NUDGE {
            for dz in -r..=r {
                for dx in -r..=r {
                    if dx.abs() != r && dz.abs() != r {
                        continue;
                    }
                    let (cx, cz) = (gcx + dx, gcz + dz);
                    let Some(j) = self.local(cx, cz) else {
                        continue;
                    };
                    if self.cell(gr, j, cx, cz) & F_BLOCKED == 0 {
                        return (cx, cz);
                    }
                }
            }
        }
        (gcx, gcz)
    }

    /// One edge, `a` to `b`: open, climbable, and not through a wall. A
    /// diagonal also needs both straight cells beside it.
    fn edge(&mut self, gr: &mut Ground, acx: i32, acz: i32, bcx: i32, bcz: i32) -> bool {
        let (Some(ai), Some(bi)) = (self.local(acx, acz), self.local(bcx, bcz)) else {
            return false;
        };
        let diag = acx != bcx && acz != bcz;
        if diag && !(self.edge(gr, acx, acz, bcx, acz) && self.edge(gr, acx, acz, acx, bcz)) {
            return false;
        }
        let fb = self.cell(gr, bi, bcx, bcz);
        if fb & F_BLOCKED != 0 {
            return false;
        }
        let ya = self.y[ai];
        let yb = self.y[bi];
        let rise = yb - ya;
        if rise < -MAX_DROP_M {
            return false;
        }
        let run = if diag { 1.414 * NAV_CELL_M } else { NAV_CELL_M };
        let climb = if fb & F_HARD != 0 {
            rise <= STEP_UP
        } else {
            rise <= run * CLIFF_SLOPE_RATIO
        };
        if !climb {
            return false;
        }
        !collide::blocked(
            gr.seed,
            gr.haven,
            gr.cols,
            centre(acx),
            centre(acz),
            centre(bcx),
            centre(bcz),
            ya,
        )
    }

    /// Walk the cells a straight segment crosses, every step an edge. Exact
    /// corner crossings step diagonally, which `edge` makes take both sides.
    fn line(&mut self, gr: &mut Ground, acx: i32, acz: i32, bcx: i32, bcz: i32) -> bool {
        let (nx, nz) = ((bcx - acx).abs(), (bcz - acz).abs());
        let (sx, sz) = ((bcx - acx).signum(), (bcz - acz).signum());
        let (mut x, mut z) = (acx, acz);
        let (mut ix, mut iz) = (0, 0);
        while ix < nx || iz < nz {
            if self.pull_left == 0 {
                return false;
            }
            self.pull_left -= 1;
            // Which boundary the segment crosses next: compare (ix+½)/nx
            // against (iz+½)/nz, cross-multiplied so it stays integer.
            let t = (1 + 2 * ix) * nz - (1 + 2 * iz) * nx;
            let (px, pz) = (x, z);
            if t == 0 {
                x += sx;
                z += sz;
                ix += 1;
                iz += 1;
            } else if t < 0 {
                x += sx;
                ix += 1;
            } else {
                z += sz;
                iz += 1;
            }
            if !self.edge(gr, px, pz, x, z) {
                return false;
            }
        }
        true
    }

    /// A* from `si` toward `gi`. Returns the cell it ended on — the goal, or
    /// the reached cell nearest it — whether that is the goal, and whether
    /// the frontier ran dry (so nothing more in the window could reach it).
    fn astar(
        &mut self,
        gr: &mut Ground,
        si: usize,
        gi: usize,
        gcx: i32,
        gcz: i32,
        cap: u32,
    ) -> (usize, bool, bool) {
        let h0 = octile(self.cx_of(si) - gcx, self.cz_of(si) - gcz);
        self.push(key(h0, h0, si));
        let (mut best, mut best_h) = (si, h0);
        let mut spent = 0u32;
        while let Some(k) = self.pop() {
            let i = (k & 0x3FFF) as usize;
            if self.flags[i] & F_CLOSED != 0 {
                continue;
            }
            self.flags[i] |= F_CLOSED;
            if i == gi {
                self.charge(spent);
                return (i, true, false);
            }
            let (cx, cz) = (self.cx_of(i), self.cz_of(i));
            let h = octile(cx - gcx, cz - gcz);
            if h < best_h {
                best = i;
                best_h = h;
            }
            spent += 1;
            if spent >= cap {
                self.charge(spent);
                return (best, false, false);
            }
            for (d, &(dx, dz)) in DIRS.iter().enumerate() {
                let (ncx, ncz) = (cx + dx, cz + dz);
                let Some(ni) = self.local(ncx, ncz) else {
                    continue;
                };
                if self.stamp[ni] == self.gen && self.flags[ni] & F_CLOSED != 0 {
                    continue;
                }
                if !self.edge(gr, cx, cz, ncx, ncz) {
                    continue;
                }
                let mut cost = self.g[i] + if d < 4 { COST_STRAIGHT } else { COST_DIAG };
                if self.flags[ni] & F_BEACH != 0 {
                    cost += COST_BEACH;
                }
                if self.flags[ni] & F_WATER != 0 {
                    cost += COST_WATER;
                }
                let rise = self.y[ni] - self.y[i];
                if rise > 0.0 {
                    cost += (rise * 10.0) as u32 * COST_CLIMB_PER_DM;
                }
                if self.flags[ni] & F_SEEN != 0 && cost >= self.g[ni] {
                    continue;
                }
                self.flags[ni] |= F_SEEN;
                self.g[ni] = cost;
                self.parent[ni] = d as u8;
                let nh = octile(ncx - gcx, ncz - gcz);
                self.push(key(cost + nh, nh, ni));
            }
        }
        self.charge(spent);
        (best, false, true)
    }

    fn charge(&mut self, spent: u32) {
        self.budget = self.budget.saturating_sub(spent);
        self.expanded += spent as u64;
    }

    /// Cells from start to `end`, into `chain`, start first. Returns the
    /// count.
    fn reconstruct(&mut self, end: usize, si: usize) -> usize {
        let mut n = 0;
        let mut i = end;
        loop {
            self.chain[n] = i as u16;
            n += 1;
            if i == si || n >= WIN_CELLS {
                break;
            }
            let d = self.parent[i];
            if d >= PARENT_NONE {
                break;
            }
            let (dx, dz) = DIRS[d as usize];
            let (cx, cz) = (self.cx_of(i) - dx, self.cz_of(i) - dz);
            match self.local(cx, cz) {
                Some(p) => i = p,
                None => break,
            }
        }
        self.chain[..n].reverse();
        n
    }

    /// Straighten a cell chain into corners: keep a cell only where the
    /// straight line from the last kept one stops being walkable, so every
    /// leg between two corners is a line `line` walked. At most
    /// `NAV_MAX_CORNERS`; a longer route is cut at the last corner that fits
    /// and comes back truncated, for the brain to finish from there.
    /// Returns how many were written and whether it was cut.
    fn pull(
        &mut self,
        gr: &mut Ground,
        n: usize,
        out: &mut [(i32, i32); NAV_MAX_CORNERS],
    ) -> (usize, bool) {
        let at = |nav: &Self, k: usize| {
            let i = nav.chain[k] as usize;
            (nav.cx_of(i), nav.cz_of(i))
        };
        let mut k = 0;
        let mut anchor = at(self, 0);
        let mut j = 2;
        while j < n {
            let (cx, cz) = at(self, j);
            if !self.line(gr, anchor.0, anchor.1, cx, cz) {
                // The line to j-1 held (it was tested last round, or it is
                // one A* edge), so j-1 is a corner.
                anchor = at(self, j - 1);
                out[k] = anchor;
                k += 1;
                if k == NAV_MAX_CORNERS {
                    return (k, true);
                }
            }
            j += 1;
        }
        out[k] = at(self, n - 1);
        (k + 1, false)
    }

    #[inline]
    fn cx_of(&self, i: usize) -> i32 {
        self.ox + (i % NAV_WIN) as i32
    }

    #[inline]
    fn cz_of(&self, i: usize) -> i32 {
        self.oz + (i / NAV_WIN) as i32
    }

    fn push(&mut self, k: u64) {
        if self.heap_len >= NAV_HEAP_CAP {
            return; // a full frontier forgets the newest cell, never panics
        }
        let mut i = self.heap_len;
        self.heap[i] = k;
        self.heap_len += 1;
        while i > 0 {
            let p = (i - 1) / 2;
            if self.heap[p] <= self.heap[i] {
                break;
            }
            self.heap.swap(p, i);
            i = p;
        }
    }

    fn pop(&mut self) -> Option<u64> {
        if self.heap_len == 0 {
            return None;
        }
        let top = self.heap[0];
        self.heap_len -= 1;
        self.heap[0] = self.heap[self.heap_len];
        let mut i = 0;
        loop {
            let (l, r) = (2 * i + 1, 2 * i + 2);
            let mut m = i;
            if l < self.heap_len && self.heap[l] < self.heap[m] {
                m = l;
            }
            if r < self.heap_len && self.heap[r] < self.heap[m] {
                m = r;
            }
            if m == i {
                break;
            }
            self.heap.swap(i, m);
            i = m;
        }
        Some(top)
    }
}

/// Probe one nav cell: can a capsule stand at its centre, and at what
/// height. The standing height is the terrain or a hard top within a step
/// of it (a foundation, a crate lid), which is `movement::step`'s ground.
fn probe(gr: &mut Ground, cx: i32, cz: i32) -> (u8, f32) {
    let (x, z) = (centre(cx), centre(cz));
    if x < BORDER_MARGIN
        || z < BORDER_MARGIN
        || x > ISLAND_SIZE - BORDER_MARGIN
        || z > ISLAND_SIZE - BORDER_MARGIN
    {
        return (F_PROBED | F_BLOCKED, 0.0);
    }
    let gy = terrain::ground(gr.seed, gr.haven, x, z);
    let hard = collide::piece_ground(gr.seed, gr.haven, gr.cols, x, z, gy)
        .max(gr.occ.ground(gr.seed, x, z, gy));
    let (y, mut f) = if hard > gy {
        (hard, F_PROBED | F_HARD)
    } else {
        (gy, F_PROBED)
    };
    if gy <= terrain::BEACH_MAX_H {
        f |= F_BEACH;
    }
    if gy <= terrain::LAND_MIN_H {
        f |= F_WATER;
    }
    if gr.occ.blocks(gr.seed, x, z, y)
        || collide::deploy_blocked(gr.seed, gr.haven, gr.cols, x, z, y)
        || collide::plane_blocked(gr.seed, gr.haven, gr.cols, x, z, y)
    {
        f |= F_BLOCKED;
    }
    (f, y)
}

fn write_path(
    out: &mut NavPath,
    corners: &[(i32, i32)],
    goal_q: (i32, i32),
    stop_cm: u16,
    partial: bool,
) {
    let n = corners.len().min(NAV_MAX_CORNERS);
    for (k, &(cx, cz)) in corners[..n].iter().enumerate() {
        out.cx[k] = cx.clamp(0, u16::MAX as i32) as u16;
        out.cz[k] = cz.clamp(0, u16::MAX as i32) as u16;
    }
    out.len = n as u8;
    out.next = 0;
    out.goal_qx = goal_q.0;
    out.goal_qz = goal_q.1;
    out.stop_cm = stop_cm;
    out.partial = partial;
    out.arrived = false;
}

/// Where the body should head now: the current corner (the exact goal for
/// the last one), advancing past corners it has reached. `None` once the
/// path is done — and the arrival is recorded on the path.
pub fn waypoint(path: &mut NavPath, qx: i32, qz: i32) -> Option<(i32, i32)> {
    while path.active() {
        let last = path.next + 1 == path.len;
        let (tx, tz) = if last {
            (path.goal_qx, path.goal_qz)
        } else {
            let k = path.next as usize;
            (
                movement_q(centre(path.cx[k] as i32)),
                movement_q(centre(path.cz[k] as i32)),
            )
        };
        let d2 = dist2_cm(tx - qx, tz - qz);
        let reach = if last {
            (path.stop_cm as i64).max(MIN_STOP_CM)
        } else {
            CORNER_REACH_CM
        };
        if d2 <= reach * reach {
            path.next += 1;
            if last {
                path.arrived = true;
            }
            continue;
        }
        return Some((tx, tz));
    }
    None
}

/// Planar distance² in centimetres² from a delta in position quanta — the
/// AOI convention (3 cm quanta, i64).
#[inline]
pub fn dist2_cm(dqx: i32, dqz: i32) -> i64 {
    let (x, z) = (dqx as i64 * 3, dqz as i64 * 3);
    x * x + z * z
}

/// The LUT entry nearest a direction — the inverse `yaw_dir` does not have,
/// and deliberately not an `atan2`: the sim's direction space *is* the
/// 256-entry table, so the entry with the largest dot product is the right
/// answer rather than an approximation of one. A zero direction keeps
/// `current`.
pub fn yaw_toward(dx: f32, dz: f32, current: u16) -> u16 {
    if dx * dx + dz * dz <= 0.0 {
        return current;
    }
    let mut best = current;
    let mut best_dot = f32::NEG_INFINITY;
    for i in 0..256u16 {
        let (ex, ez) = yaw_dir(i << 8);
        let dot = ex * dx + ez * dz;
        if dot > best_dot {
            best_dot = dot;
            best = i << 8;
        }
    }
    best
}

/// The unsigned angle between two headings, in yaw units (65536 = a turn).
#[inline]
pub fn yaw_gap(a: u16, b: u16) -> u16 {
    (b.wrapping_sub(a) as i16).unsigned_abs()
}

/// Turn `cur` toward `want` by at most `step`, the short way round.
#[inline]
pub fn turn_toward(cur: u16, want: u16, step: u16) -> u16 {
    let d = want.wrapping_sub(cur) as i16;
    if d.unsigned_abs() <= step {
        want
    } else if d > 0 {
        cur.wrapping_add(step)
    } else {
        cur.wrapping_sub(step)
    }
}

#[inline]
fn cell_of(m: f32) -> i32 {
    floor_i32(m / NAV_CELL_M)
}

#[inline]
fn centre(c: i32) -> f32 {
    (c as f32 + 0.5) * NAV_CELL_M
}

#[inline]
fn movement_q(m: f32) -> i32 {
    floor_i32(m / POS_XZ_Q + 0.5)
}

#[inline]
fn octile(dx: i32, dz: i32) -> u32 {
    let (ax, az) = (dx.unsigned_abs(), dz.unsigned_abs());
    let (lo, hi) = if ax < az { (ax, az) } else { (az, ax) };
    COST_STRAIGHT * hi + (COST_DIAG - COST_STRAIGHT) * lo
}

/// Heap key: lower f first, then lower h (nearer the goal), then the lower
/// window index, so equal-cost ties break the same way on every build.
#[inline]
fn key(f: u32, h: u32, i: usize) -> u64 {
    ((f.min(0x3FFF_FFFF) as u64) << 34) | ((h.min(0xF_FFFF) as u64) << 14) | i as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turning_takes_the_short_way_and_stops_on_the_heading() {
        assert_eq!(turn_toward(0, 10 << 8, 4 << 8), 4 << 8);
        assert_eq!(turn_toward(0, 250 << 8, 4 << 8), 252 << 8);
        assert_eq!(turn_toward(100, 104, 8), 104);
        assert_eq!(yaw_gap(0, 128 << 8), 128 << 8);
        assert_eq!(yaw_gap(250 << 8, 2 << 8), 8 << 8);
    }

    #[test]
    fn the_heap_pops_in_key_order() {
        let mut n = Nav::new();
        for k in [5u64, 1, 9, 3, 7, 2, 8] {
            n.push(k);
        }
        let mut got = [0u64; 7];
        for g in got.iter_mut() {
            *g = n.pop().unwrap();
        }
        assert_eq!(got, [1, 2, 3, 5, 7, 8, 9]);
        assert!(n.pop().is_none());
    }

    #[test]
    fn octile_is_the_diagonal_metric() {
        assert_eq!(octile(3, 0), 30);
        assert_eq!(octile(3, 3), 42);
        assert_eq!(octile(-2, 5), 58);
    }
}
