//! The long way round. The route's metre grid (`sim_core::nav`) plans in a
//! 96 m window, toward a goal past it along the straight line, with a few
//! hundred cells to spend. A cliff face across that line wider than that
//! is a wall it cannot see the end of: from the foot of one, or a ledge on
//! one, nothing it can reach is any closer, and the route says there is no
//! way at once. A walk home from the far side of a ridge failed in 0 s
//! every time it was offered, and the body starved with its food in its
//! box (a report, 2026-10-19).
//!
//! [`Far`] does what a player does with the map: a coarse A* over the
//! terrain alone, [`FAR_CELL_M`] cells over a window [`FAR_WIN`] of them
//! across: a cell open where its middle is not a face (`face::steep`, the
//! metre grid's own test), a step between two open where the ground
//! between them is no steeper than one, and a body on a face sliding down
//! it as the metre grid lets it. It finds the way round; the route walks it on
//! the metre grid a waypoint at a time. Pieces, rocks and crags are the
//! metre grid's to see: a waypoint it cannot get to is marked
//! ([`Far::bar`]) and the way is found again round it.
//!
//! Bounded like the metre grid: [`FAR_EXPAND_PER_FRAME`] cells a frame,
//! [`FAR_MAX_EXPAND`] a search, on scratch allocated once with the body.

use sim_core::movement::BORDER_MARGIN;
use sim_core::nav::Ground;
use sim_core::terrain::{self, CLIFF_SLOPE_RATIO, ISLAND_SIZE, LAND_MIN_H};

/// A coarse cell's side, metres.
pub const FAR_CELL_M: f32 = 4.0;
/// Cells on a side of the window: 640 m.
pub const FAR_WIN: usize = 160;
const CELLS: usize = FAR_WIN * FAR_WIN;
const _: () = assert!(
    CELLS <= 1 << 15,
    "a window index must fit the heap key's 15 bits"
);
/// Cells expanded a frame, and in a whole search.
pub const FAR_EXPAND_PER_FRAME: u32 = 192;
pub const FAR_MAX_EXPAND: u32 = 8192;
/// The frontier's room; full, the newest cell is forgotten.
const HEAP_CAP: usize = 4 * FAR_MAX_EXPAND as usize;
/// Waypoints a way keeps, and how many cells apart along it they are at
/// least.
pub const FAR_PTS: usize = 48;
const STRIDE: usize = 3;
/// A waypoint is reached within a cell of it: one in a tree or a rock is
/// stopped short of by the metre grid.
pub const FAR_REACH_M: f32 = FAR_CELL_M;
/// Waypoints the metre grid could not get to, kept for the goal in hand.
pub const FAR_BARRED: usize = 8;
/// The ground between two cells is walked where it is no steeper than
/// this, rise over run, half a step at a time. A face is
/// `CLIFF_SLOPE_RATIO` over three metres, and coarse heights round off a
/// face's lip and foot, so this keeps a margin.
const SLOPE: f32 = CLIFF_SLOPE_RATIO * 0.8;
/// A slide onto more face drops at least this, rise over run: near
/// enough its fall line.
const FALL: f32 = 0.6;
/// Step costs, tenths of a cell; water is waded, at half speed and only
/// when the way round is a good deal longer; a climb costs a little.
const COST_STRAIGHT: u32 = 10;
const COST_DIAG: u32 = 14;
const COST_WATER: u32 = 30;
const COST_CLIMB_PER_M: u32 = 2;
/// A way that ends short of the goal is kept only if it ends this much
/// closer (heuristic units: tenths of a cell).
const MIN_GAIN: u32 = 3 * COST_STRAIGHT;

const F_SEEN: u8 = 1;
const F_CLOSED: u8 = 2;
const F_HEIGHT: u8 = 4;
/// Whether the cell's middle is a face is known, and it is.
const F_FACED: u8 = 8;
const F_FACE: u8 = 16;
/// Reached by sliding down a face (or the start): downhill is open from
/// here, face or not.
const F_SLIDE: u8 = 32;
const PARENT_NONE: u8 = 8;
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

/// Where a search stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Search {
    /// Nothing in hand.
    Idle,
    /// Under way: [`Far::run`] it again next frame.
    Running,
    /// A way: walk [`Far::point`] after point.
    Found,
    /// No way round within the window.
    NoWay,
}

pub struct Far {
    stamp: Box<[u16; CELLS]>,
    flags: Box<[u8; CELLS]>,
    y: Box<[f32; CELLS]>,
    g: Box<[u32; CELLS]>,
    parent: Box<[u8; CELLS]>,
    heap: Box<[u64; HEAP_CAP]>,
    heap_len: usize,
    gen: u16,
    /// The window's low corner, in island cells.
    ox: i32,
    oz: i32,
    start: usize,
    goal: usize,
    best: usize,
    best_h: u32,
    start_h: u32,
    spent: u32,
    state: Search,
    pts: [[f32; 2]; FAR_PTS],
    n: usize,
    at: usize,
    barred: [Option<(i32, i32)>; FAR_BARRED],
    barred_next: usize,
    /// What the search is for, metres.
    target: [f32; 2],
}

impl Default for Far {
    fn default() -> Self {
        Self::new()
    }
}

impl Far {
    pub fn new() -> Self {
        Self {
            stamp: boxed(0),
            flags: boxed(0),
            y: boxed(0.0),
            g: boxed(0),
            parent: boxed(0),
            heap: boxed(0),
            heap_len: 0,
            gen: 0,
            ox: 0,
            oz: 0,
            start: 0,
            goal: 0,
            best: 0,
            best_h: 0,
            start_h: 0,
            spent: 0,
            state: Search::Idle,
            pts: [[0.0; 2]; FAR_PTS],
            n: 0,
            at: 0,
            barred: [None; FAR_BARRED],
            barred_next: 0,
            target: [0.0; 2],
        }
    }

    pub fn state(&self) -> Search {
        self.state
    }

    /// What the search in hand is for.
    pub fn target(&self) -> [f32; 2] {
        self.target
    }

    /// Forget the way and the waypoints barred for it: a new goal.
    pub fn clear(&mut self) {
        self.state = Search::Idle;
        self.n = 0;
        self.at = 0;
        self.barred = [None; FAR_BARRED];
        self.barred_next = 0;
    }

    /// The way is walked: nothing in hand, the barred waypoints kept.
    pub fn done(&mut self) {
        self.state = Search::Idle;
        self.n = 0;
        self.at = 0;
    }

    /// Start a search from a body standing at `from` (`x, y, z`) toward
    /// `to`; [`Far::run`] does the work.
    pub fn begin(&mut self, from: [f32; 3], to: [f32; 2]) {
        self.gen = self.gen.wrapping_add(1);
        if self.gen == 0 {
            self.stamp.fill(0);
            self.gen = 1;
        }
        self.heap_len = 0;
        self.spent = 0;
        self.n = 0;
        self.at = 0;
        self.target = to;
        let (scx, scz) = (cell_of(from[0]), cell_of(from[2]));
        let (gcx, gcz) = (cell_of(to[0]), cell_of(to[1]));
        let half = FAR_WIN as i32 / 2;
        let margin = FAR_WIN as i32 / 8;
        let fits = (gcx - scx).abs() <= FAR_WIN as i32 - 2 * margin
            && (gcz - scz).abs() <= FAR_WIN as i32 - 2 * margin;
        let (mx, mz) = if fits {
            ((scx + gcx) / 2, (scz + gcz) / 2)
        } else {
            (scx, scz)
        };
        self.ox = mx - half;
        self.oz = mz - half;
        let (gcx, gcz) = if self.local(gcx, gcz).is_some() {
            (gcx, gcz)
        } else {
            self.clip(scx, scz, gcx, gcz)
        };
        let (Some(si), Some(gi)) = (self.local(scx, scz), self.local(gcx, gcz)) else {
            self.state = Search::NoWay;
            return;
        };
        self.touch(si);
        // The start is where the body stands, at its feet's height: a
        // ledge on a face is somewhere to slide down from.
        self.y[si] = from[1];
        self.flags[si] |= F_HEIGHT | F_SEEN | F_SLIDE;
        self.g[si] = 0;
        self.parent[si] = PARENT_NONE;
        self.start = si;
        self.goal = gi;
        let h = self.h(si);
        self.start_h = h;
        self.best = si;
        self.best_h = h;
        self.push(key(h, h, si));
        self.state = Search::Running;
    }

    /// Expand up to `budget` cells of the search in hand.
    pub fn run(&mut self, gr: &mut Ground, budget: u32) {
        if self.state != Search::Running {
            return;
        }
        let mut left = budget;
        while left > 0 {
            let Some(k) = self.pop() else {
                return self.settle();
            };
            let i = (k & 0x7FFF) as usize;
            if self.flags[i] & F_CLOSED != 0 {
                continue;
            }
            self.flags[i] |= F_CLOSED;
            if i == self.goal {
                self.best = i;
                return self.trace(i);
            }
            let h = self.h(i);
            if h < self.best_h {
                self.best = i;
                self.best_h = h;
            }
            self.spent += 1;
            left -= 1;
            if self.spent >= FAR_MAX_EXPAND {
                return self.settle();
            }
            let (cx, cz) = self.cell(i);
            let ya = self.height(gr, i);
            let slides = self.flags[i] & F_SLIDE != 0;
            for (d, &(dx, dz)) in DIRS.iter().enumerate() {
                let (ncx, ncz) = (cx + dx, cz + dz);
                let Some(ni) = self.local(ncx, ncz) else {
                    continue;
                };
                self.touch(ni);
                if self.flags[ni] & F_CLOSED != 0 || self.is_barred(ncx, ncz) {
                    continue;
                }
                let (nx, nz) = (centre(ncx), centre(ncz));
                if nx < BORDER_MARGIN
                    || nz < BORDER_MARGIN
                    || nx > ISLAND_SIZE - BORDER_MARGIN
                    || nz > ISLAND_SIZE - BORDER_MARGIN
                {
                    continue;
                }
                let yb = self.height(gr, ni);
                let face = self.face(gr, ni);
                let mid = terrain::ground(
                    gr.seed,
                    gr.haven,
                    (centre(cx) + nx) * 0.5,
                    (centre(cz) + nz) * 0.5,
                );
                let run = if d < 4 {
                    FAR_CELL_M
                } else {
                    FAR_CELL_M * 1.414
                };
                let step = |a: f32, b: f32| (b - a).abs() <= run * 0.5 * SLOPE;
                // Nobody walks onto a face; one already on a face slides
                // down it, off it, or onto more face down its fall line
                // (the metre grid takes no step up on a face, and across
                // one the ground's own roughness is a step up).
                let fall = if face { run * 0.5 * FALL } else { 0.0 };
                let down = slides && mid <= ya - fall && yb <= mid - fall;
                if !(down || (!face && step(ya, mid) && step(mid, yb))) {
                    continue;
                }
                let mut cost = self.g[i] + if d < 4 { COST_STRAIGHT } else { COST_DIAG };
                if yb <= LAND_MIN_H {
                    cost += COST_WATER;
                }
                if yb > ya {
                    cost += (yb - ya) as u32 * COST_CLIMB_PER_M;
                }
                if self.flags[ni] & F_SEEN != 0 && cost >= self.g[ni] {
                    continue;
                }
                self.flags[ni] |= F_SEEN;
                if face {
                    self.flags[ni] |= F_SLIDE;
                } else {
                    self.flags[ni] &= !F_SLIDE;
                }
                self.g[ni] = cost;
                self.parent[ni] = d as u8;
                let nh = self.h(ni);
                self.push(key(cost + nh, nh, ni));
            }
        }
    }

    /// The next waypoint, metres, if the way has one left.
    pub fn point(&self) -> Option<[f32; 2]> {
        (self.state == Search::Found && self.at < self.n).then(|| self.pts[self.at])
    }

    /// On to the waypoint after.
    pub fn advance(&mut self) {
        self.at = (self.at + 1).min(self.n);
    }

    /// The metre grid could not get to the waypoint at `p`: the next
    /// search goes round it.
    pub fn bar(&mut self, p: [f32; 2]) {
        self.barred[self.barred_next] = Some((cell_of(p[0]), cell_of(p[1])));
        self.barred_next = (self.barred_next + 1) % FAR_BARRED;
    }

    fn is_barred(&self, cx: i32, cz: i32) -> bool {
        self.barred.contains(&Some((cx, cz)))
    }

    /// Out of frontier or of budget: the way to the cell nearest the goal,
    /// if that is worth walking.
    fn settle(&mut self) {
        if self.best != self.start && self.best_h + MIN_GAIN <= self.start_h {
            self.trace(self.best);
        } else {
            self.state = Search::NoWay;
        }
    }

    /// The way to `end` as waypoints, start first, at least [`STRIDE`]
    /// cells apart, the end always among them.
    fn trace(&mut self, end: usize) {
        let mut len = 0usize;
        let mut i = end;
        while i != self.start && len < CELLS {
            let Some(p) = self.step_back(i) else {
                break;
            };
            i = p;
            len += 1;
        }
        if len == 0 {
            self.state = Search::NoWay;
            return;
        }
        let stride = STRIDE.max(len.div_ceil(FAR_PTS));
        // Cells along the way, the start at 0 and `end` at `len`: the
        // waypoints are every `stride`-th, and the end.
        let n = len.div_ceil(stride);
        let mut i = end;
        let mut k = len;
        loop {
            if k == len {
                self.pts[n - 1] = self.centre_of(i);
            } else if k > 0 && k.is_multiple_of(stride) {
                self.pts[k / stride - 1] = self.centre_of(i);
            }
            if k == 0 {
                break;
            }
            let Some(p) = self.step_back(i) else {
                break;
            };
            i = p;
            k -= 1;
        }
        self.n = n;
        self.at = 0;
        self.state = Search::Found;
    }

    fn step_back(&self, i: usize) -> Option<usize> {
        let d = self.parent[i];
        if d == PARENT_NONE {
            return None;
        }
        let (dx, dz) = DIRS[usize::from(d)];
        let (cx, cz) = self.cell(i);
        self.local(cx - dx, cz - dz)
    }

    fn centre_of(&self, i: usize) -> [f32; 2] {
        let (cx, cz) = self.cell(i);
        [centre(cx), centre(cz)]
    }

    fn touch(&mut self, i: usize) {
        if self.stamp[i] != self.gen {
            self.stamp[i] = self.gen;
            self.flags[i] = 0;
        }
    }

    fn height(&mut self, gr: &mut Ground, i: usize) -> f32 {
        self.touch(i);
        if self.flags[i] & F_HEIGHT == 0 {
            let (cx, cz) = self.cell(i);
            self.y[i] = terrain::ground(gr.seed, gr.haven, centre(cx), centre(cz));
            self.flags[i] |= F_HEIGHT;
        }
        self.y[i]
    }

    /// The cell's middle is a face, as the metre grid tells one.
    fn face(&mut self, gr: &mut Ground, i: usize) -> bool {
        self.touch(i);
        if self.flags[i] & F_FACED == 0 {
            let (cx, cz) = self.cell(i);
            if sim_core::face::steep(gr.occ, gr.seed, gr.haven, centre(cx), centre(cz)) {
                self.flags[i] |= F_FACE;
            }
            self.flags[i] |= F_FACED;
        }
        self.flags[i] & F_FACE != 0
    }

    fn h(&self, i: usize) -> u32 {
        let (cx, cz) = self.cell(i);
        let (gx, gz) = self.cell(self.goal);
        octile(cx - gx, cz - gz)
    }

    fn cell(&self, i: usize) -> (i32, i32) {
        (
            self.ox + (i % FAR_WIN) as i32,
            self.oz + (i / FAR_WIN) as i32,
        )
    }

    fn local(&self, cx: i32, cz: i32) -> Option<usize> {
        let (lx, lz) = (cx - self.ox, cz - self.oz);
        if lx < 0 || lz < 0 || lx >= FAR_WIN as i32 || lz >= FAR_WIN as i32 {
            return None;
        }
        Some(lz as usize * FAR_WIN + lx as usize)
    }

    /// The last cell on the line from the start toward a goal outside the
    /// window, one cell in from its edge.
    fn clip(&self, scx: i32, scz: i32, gcx: i32, gcz: i32) -> (i32, i32) {
        let (dx, dz) = ((gcx - scx) as f32, (gcz - scz) as f32);
        let (lo_x, hi_x) = ((self.ox + 1) as f32, (self.ox + FAR_WIN as i32 - 2) as f32);
        let (lo_z, hi_z) = ((self.oz + 1) as f32, (self.oz + FAR_WIN as i32 - 2) as f32);
        let (sx, sz) = (scx as f32, scz as f32);
        let mut t = 1.0f32;
        if dx > 0.0 && sx + dx * t > hi_x {
            t = t.min((hi_x - sx) / dx);
        }
        if dx < 0.0 && sx + dx * t < lo_x {
            t = t.min((lo_x - sx) / dx);
        }
        if dz > 0.0 && sz + dz * t > hi_z {
            t = t.min((hi_z - sz) / dz);
        }
        if dz < 0.0 && sz + dz * t < lo_z {
            t = t.min((lo_z - sz) / dz);
        }
        let t = t.max(0.0);
        (
            ((sx + dx * t).round() as i32).clamp(self.ox + 1, self.ox + FAR_WIN as i32 - 2),
            ((sz + dz * t).round() as i32).clamp(self.oz + 1, self.oz + FAR_WIN as i32 - 2),
        )
    }

    fn push(&mut self, k: u64) {
        if self.heap_len >= HEAP_CAP {
            return;
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

/// A fixed array on the heap, filled: built once, with the body.
fn boxed<T: Clone, const N: usize>(fill: T) -> Box<[T; N]> {
    let Ok(b) = vec![fill; N].into_boxed_slice().try_into() else {
        unreachable!("the vec is N long by construction")
    };
    b
}

fn cell_of(m: f32) -> i32 {
    (m / FAR_CELL_M).floor() as i32
}

fn centre(c: i32) -> f32 {
    (c as f32 + 0.5) * FAR_CELL_M
}

fn octile(dx: i32, dz: i32) -> u32 {
    let (ax, az) = (dx.unsigned_abs(), dz.unsigned_abs());
    let (lo, hi) = if ax < az { (ax, az) } else { (az, ax) };
    COST_STRAIGHT * hi + (COST_DIAG - COST_STRAIGHT) * lo
}

/// Heap key: lower f first, then lower h, then the lower window index.
fn key(f: u32, h: u32, i: usize) -> u64 {
    ((f.min(0x3FFF_FFFF) as u64) << 34) | ((h.min(0x7_FFFF) as u64) << 15) | i as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: u64 = 20260731;

    /// The report's ground (seed 20260731): a body on a cliff face whose
    /// home lies up and past it. From there the metre grid finds nothing
    /// closer to anywhere north-east (`sim_core::nav` says `NoWay`), and
    /// the coarse search finds the way round: down off the face and round
    /// the end of it, every step of it no steeper than a face, ending
    /// nearer the goal than it began.
    #[test]
    fn a_body_on_a_cliff_face_is_shown_the_way_round_it() {
        let haven = terrain::haven(SEED);
        let from = [2810.5, 56.8, 2594.5];
        let to = [3088.0, 2709.0];
        let mut sc = sim_core::occupy::Scratch::live(SEED);
        let cols = Box::new(sim_core::collide::ColIndex::new());
        let mut far = Far::new();
        far.begin(from, to);
        let mut frames = 0;
        while far.state() == Search::Running {
            let mut occ = sc.occupants();
            let mut gr = Ground {
                seed: SEED,
                haven: &haven,
                cols: &cols,
                occ: &mut occ,
            };
            far.run(&mut gr, FAR_EXPAND_PER_FRAME);
            frames += 1;
            assert!(frames < 200, "bounded");
        }
        assert_eq!(far.state(), Search::Found);
        // The metre grid, as the route walks it: from the body, then from
        // each waypoint to the next, every leg a whole plan (the island's
        // rocks and trees in the way, nothing built).
        let mut nav = Box::new(sim_core::nav::Nav::new());
        let mut path = sim_core::nav::NavPath::default();
        let mut leg = |a: [f32; 3], b: [f32; 2]| {
            nav.begin_tick();
            let mut occ = sc.occupants();
            let mut gr = Ground {
                seed: SEED,
                haven: &haven,
                cols: &cols,
                occ: &mut occ,
            };
            let plan = nav.plan(&mut gr, a[0], a[1], a[2], b[0], b[1], 100, &mut path);
            let q = sim_core::movement::POS_XZ_Q;
            let end = [path.goal_qx as f32 * q, path.goal_qz as f32 * q];
            (plan, end)
        };
        let (blocked, _) = leg(from, to);
        assert_eq!(blocked, sim_core::nav::Plan::NoWay, "the report's dead end");
        let d = |p: [f32; 2]| (p[0] - to[0]).hypot(p[1] - to[1]);
        let mut last = [from[0], from[2]];
        let mut at = from;
        let mut n = 0;
        while let Some(p) = far.point() {
            assert!(
                (p[0] - last[0]).hypot(p[1] - last[1]) <= 3.0 * STRIDE as f32 * FAR_CELL_M,
                "waypoints a short walk apart: {last:?} {p:?}"
            );
            // To the waypoint, or as near as the route stops for one, and
            // on from where it stopped.
            let (plan, end) = leg(at, p);
            let short = (end[0] - p[0]).hypot(end[1] - p[1]);
            assert!(
                plan.moving() && short <= FAR_REACH_M,
                "waypoint {n} {p:?} from {at:?}: {plan:?}, {short} m short"
            );
            last = p;
            at = [
                end[0],
                terrain::ground(SEED, &haven, end[0], end[1]),
                end[1],
            ];
            far.advance();
            n += 1;
        }
        assert!(n > 0);
        assert!(
            d(last) + 20.0 < d([from[0], from[2]]),
            "it ends nearer the goal: {last:?}"
        );
    }
}
