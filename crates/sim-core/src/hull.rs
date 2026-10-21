//! Built pieces as collision answers them, one row per piece in its own
//! frame (freeform W1).
//!
//! The column index (`collide::ColIndex`) knows a piece as a bit in a 3 m
//! cell, which is every piece's shape only while every piece is a cell or
//! one of its sides. Here a piece is a [`Hull`]: an origin, two unit axes
//! and a kind, so a wall at 30° and a wall along the grid are one
//! narrowphase, and the broadphase is [`space::Space`]'s 4 m buckets.
//!
//! **A grid piece's frame is its cell's**: the origin on the cell's low
//! corner (or the edge's start) and the axes along x and z, so every local
//! coordinate is the same subtraction `collide.rs` makes (`x - cx·3`), and
//! the per-shape tests (`floor_frame_solid`, `tri_half_has`,
//! `circulation::surface`, `edge_meet`) run unchanged on it. Multiplying
//! by 1 and adding a signed zero are exact, so the answers are the column
//! index's to the bit — which is what `tests/hull_shadow.rs` holds them to.
//!
//! Every query is order-independent (any, min, max, or a minimum by
//! [`Hull::rank`]), because [`Space`] hands rows back in table order and
//! table order is history: a restored shard and a client mirror fill it
//! differently.

use crate::build::{
    LEVEL_H_M, LOC_DIAG_B, LOC_EDGE_XLO, LOC_EDGE_ZLO, LOC_PLANE, LOC_TRI_XHI_ZHI, LOC_TRI_XHI_ZLO,
    LOC_TRI_XLO_ZHI, LOC_TRI_XLO_ZLO, SHAPE_DOORWAY, SHAPE_FOUNDATION_STEPS, SHAPE_FRAME,
    SHAPE_WINDOW,
};
use crate::collide::{
    doorway_solid_at, floor_frame_solid, frame_solid_at, tri_frame_solid, tri_half_has,
    window_solid_at, PieceHit, CAPSULE_HEIGHT_M, CAPSULE_RADIUS_M, DOOR_HEAD_M, FRAME_RIM_M,
    NO_SURFACE, PLANE_THICKNESS_M, WALL_THICKNESS_M,
};
use crate::fmath::fabs;
use crate::limits::{MAX_PIECES, PIECE_ADDR_SLOTS};
use crate::movement::STEP_UP;
use crate::space::{Space, SPACE_BUCKET_M, SPACE_SIDE};

/// A square body: foundation, floor, roof, floor frame.
pub const HULL_PLANE: u8 = 0;
/// The legacy right-isosceles half cell (`build::LOC_TRI_*`, in `loc`).
pub const HULL_HALF: u8 = 1;
/// A flight on a square body (`circulation`).
pub const HULL_RISER: u8 = 2;
/// A 3 m edge: wall, half wall, low wall, doorway, window, frame.
pub const HULL_EDGE: u8 = 3;
/// The legacy 4.24 m cell diagonal (`build::LOC_DIAG_*`, in `loc`).
pub const HULL_DIAG: u8 = 4;
/// An equilateral triangle body: corners (0, 0), (3, 0) and (1.5, 3√3/2)
/// in its own frame, the inside on local +z of its first side.
pub const HULL_TRI: u8 = 5;

/// √3 / 2.
const H3: f32 = 0.866_025_4;

/// Inward distances from a local point to an equilateral body's three
/// sides: the first side, then round the triangle. All three are at least
/// zero exactly inside.
#[inline]
fn tri_d(lx: f32, lz: f32) -> (f32, f32, f32) {
    (lz, H3 * (SIDE_M - lx) - 0.5 * lz, H3 * lx - 0.5 * lz)
}

/// Squared distance from a local point to an equilateral body (zero
/// inside): the nearest side, or the nearest corner past a side's ends.
fn tri_dist2(lx: f32, lz: f32) -> f32 {
    let (d0, d1, d2) = tri_d(lx, lz);
    if d0 >= 0.0 && d1 >= 0.0 && d2 >= 0.0 {
        return 0.0;
    }
    let corners = [(0.0, 0.0), (SIDE_M, 0.0), (SIDE_M * 0.5, SIDE_M * H3)];
    let mut best = f32::MAX;
    for i in 0..3 {
        let (ax, az) = corners[i];
        let (bx, bz) = corners[(i + 1) % 3];
        let (ex, ez) = (bx - ax, bz - az);
        let t = (((lx - ax) * ex + (lz - az) * ez) / (SIDE_M * SIDE_M)).clamp(0.0, 1.0);
        let (px, pz) = (ax + t * ex - lx, az + t * ez - lz);
        best = best.min(px * px + pz * pz);
    }
    best
}

/// Whether a triangle floor frame's rim is under a probe of radius `r`:
/// within the rim of any side.
#[inline]
fn tri_rim_solid(lx: f32, lz: f32, r: f32) -> bool {
    let (d0, d1, d2) = tri_d(lx, lz);
    let rim = FRAME_RIM_M + r;
    d0 <= rim || d1 <= rim || d2 <= rim
}

/// The side of a square body and the length of a straight edge.
const SIDE_M: f32 = crate::build::BUILD_CELL_M;
const INV_SQRT2: f32 = std::f32::consts::FRAC_1_SQRT_2;
const DIAG_LEN_M: f32 = SIDE_M * std::f32::consts::SQRT_2;

/// One built piece in the world.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hull {
    /// The caller's identity for the row: `del` and `set_shut` find it by
    /// this. Never zero.
    pub key: u64,
    /// What a shot that stops on it reports.
    pub hit: PieceHit,
    pub kind: u8,
    pub shape: u8,
    /// The legacy sub-address the kind needs: a half cell's corner, a
    /// diagonal's direction, a flight's heading in its cell.
    pub loc: u8,
    /// The socket: level 0 is a foundation, solid to the ground; a slab
    /// above is `PLANE_THICKNESS_M` with air under it.
    pub level: u8,
    /// A floor frame (or its triangle): the centre is an opening.
    pub open: bool,
    /// Something closed stands in an opening (a shut door, a closed
    /// insert), and `panes` says the closed insert is glass or shutter.
    pub shut: bool,
    pub panes: bool,
    /// World y of the piece's floor (a body's top, an edge's bottom).
    pub floor_y: f32,
    /// World y an edge reaches down to (its floor less any foot drop).
    pub foot_y: f32,
    /// The local origin: a body's low corner, an edge's start.
    pub ox: f32,
    pub oz: f32,
    /// Local +x (a body's first side, an edge's run) and local +z, unit.
    pub ux: f32,
    pub uz: f32,
    pub vx: f32,
    pub vz: f32,
}

impl Hull {
    pub const EMPTY: Self = Self {
        key: 0,
        hit: PieceHit {
            cx: 0,
            cz: 0,
            level: 0,
            loc: 0,
        },
        kind: HULL_PLANE,
        shape: 0,
        loc: 0,
        level: 0,
        open: false,
        shut: false,
        panes: false,
        floor_y: 0.0,
        foot_y: 0.0,
        ox: 0.0,
        oz: 0.0,
        ux: 1.0,
        uz: 0.0,
        vx: 0.0,
        vz: 1.0,
    };

    /// A world point in the hull's own frame: (along local x, along local z).
    #[inline]
    pub fn local(&self, x: f32, z: f32) -> (f32, f32) {
        let (dx, dz) = (x - self.ox, z - self.oz);
        (dx * self.ux + dz * self.uz, dx * self.vx + dz * self.vz)
    }

    /// An edge's height above its floor.
    #[inline]
    pub fn height(&self) -> f32 {
        crate::build::wall_height(self.shape)
    }

    /// The order a shot prefers among rows it stops on at one sample:
    /// edges before diagonals before bodies, then the address, a body
    /// before the flights on it — the order `collide.rs` walked them in,
    /// as far as one key can say it.
    #[inline]
    pub fn rank(&self) -> (u8, u16, u16, u8, u8) {
        let cat = match self.kind {
            HULL_EDGE => 0,
            HULL_DIAG => 1,
            _ => 2,
        };
        let loc = match self.hit.loc {
            LOC_PLANE => 0,
            LOC_TRI_XLO_ZLO => 1,
            LOC_TRI_XHI_ZLO => 2,
            LOC_TRI_XLO_ZHI => 3,
            LOC_TRI_XHI_ZHI => 4,
            crate::build::LOC_RISER => 5,
            crate::build::LOC_RISER_XHI => 6,
            crate::build::LOC_RISER_ZLO => 7,
            crate::build::LOC_RISER_XLO => 8,
            other => 9 + other,
        };
        (cat, self.hit.cx, self.hit.cz, self.hit.level, loc)
    }

    /// The world box the row is filed under: its footprint, and for an
    /// edge half the wall's thickness either side.
    fn aabb(&self) -> (f32, f32, f32, f32) {
        if self.kind == HULL_TRI {
            let pts = [
                (self.ox, self.oz),
                (self.ox + SIDE_M * self.ux, self.oz + SIDE_M * self.uz),
                (
                    self.ox + SIDE_M * 0.5 * self.ux + SIDE_M * H3 * self.vx,
                    self.oz + SIDE_M * 0.5 * self.uz + SIDE_M * H3 * self.vz,
                ),
            ];
            let (mut x0, mut z0, mut x1, mut z1) = (pts[0].0, pts[0].1, pts[0].0, pts[0].1);
            for &(x, z) in &pts[1..] {
                x0 = x0.min(x);
                z0 = z0.min(z);
                x1 = x1.max(x);
                z1 = z1.max(z);
            }
            return (x0, z0, x1, z1);
        }
        let (len, wide, pad) = match self.kind {
            HULL_EDGE => (SIDE_M, 0.0, WALL_THICKNESS_M * 0.5),
            HULL_DIAG => {
                // The diagonal's own frame is its cell's: it spans the
                // whole cell either way.
                (SIDE_M, SIDE_M, WALL_THICKNESS_M * 0.5)
            }
            _ => (SIDE_M, SIDE_M, 0.0),
        };
        let pts = [
            (self.ox, self.oz),
            (self.ox + len * self.ux, self.oz + len * self.uz),
            (self.ox + wide * self.vx, self.oz + wide * self.vz),
            (
                self.ox + len * self.ux + wide * self.vx,
                self.oz + len * self.uz + wide * self.vz,
            ),
        ];
        let (mut x0, mut z0, mut x1, mut z1) = (pts[0].0, pts[0].1, pts[0].0, pts[0].1);
        for &(x, z) in &pts[1..] {
            x0 = x0.min(x);
            z0 = z0.min(z);
            x1 = x1.max(x);
            z1 = z1.max(z);
        }
        (x0 - pad, z0 - pad, x1 + pad, z1 + pad)
    }

    /// Squared distance from a local point to the body's square — the
    /// clamp-to-rectangle distance `collide.rs` measures a body's reach
    /// with.
    #[inline]
    fn body_dist2(lx: f32, lz: f32) -> f32 {
        let ex = lx - lx.clamp(0.0, SIDE_M);
        let ez = lz - lz.clamp(0.0, SIDE_M);
        ex * ex + ez * ez
    }

    /// Whether a body hull is solid under (`lx`, `lz`) for a probe of
    /// radius `r` (zero for a point): a square, a half's own triangle, a
    /// frame's rim. Half-open on the body's square, the column index's
    /// cell membership.
    fn body_solid(&self, lx: f32, lz: f32, r: f32) -> bool {
        match self.kind {
            HULL_PLANE => !self.open || floor_frame_solid(lx, lz, r),
            HULL_HALF => {
                tri_half_has(self.loc, lx, lz)
                    && (!self.open || tri_frame_solid(self.loc, lx, lz, r))
            }
            HULL_TRI => {
                let (d0, d1, d2) = tri_d(lx, lz);
                d0 >= 0.0 && d1 >= 0.0 && d2 >= 0.0 && (!self.open || tri_rim_solid(lx, lz, r))
            }
            _ => false,
        }
    }

    /// Whether a body's own footprint holds a local point for a point
    /// query: half-open on a square (the column index's cell membership),
    /// closed on a triangle.
    #[inline]
    fn holds(&self, lx: f32, lz: f32) -> bool {
        if self.kind == HULL_TRI {
            let (d0, d1, d2) = tri_d(lx, lz);
            d0 >= 0.0 && d1 >= 0.0 && d2 >= 0.0
        } else {
            (0.0..SIDE_M).contains(&lx) && (0.0..SIDE_M).contains(&lz)
        }
    }

    /// Squared distance from a local point to the body's footprint.
    #[inline]
    fn foot_dist2(&self, lx: f32, lz: f32) -> f32 {
        if self.kind == HULL_TRI {
            tri_dist2(lx, lz)
        } else {
            Hull::body_dist2(lx, lz)
        }
    }

    #[inline]
    pub fn is_body(&self) -> bool {
        matches!(self.kind, HULL_PLANE | HULL_HALF | HULL_RISER | HULL_TRI)
    }

    /// Whether the row is a whole slab — a square or a triangle, not a
    /// frame — whose footprint holds the world point: what a deployable
    /// stands on.
    #[inline]
    pub fn slab_holds(&self, x: f32, z: f32) -> bool {
        if !matches!(self.kind, HULL_PLANE | HULL_TRI) || self.open {
            return false;
        }
        let (lx, lz) = self.local(x, z);
        self.holds(lx, lz)
    }

    /// The row's middle, world XZ: a body's centre (a triangle's centroid),
    /// an edge's midpoint.
    pub fn middle(&self) -> (f32, f32) {
        let (a, b) = match self.kind {
            HULL_EDGE => (SIDE_M * 0.5, 0.0),
            HULL_TRI => (SIDE_M * 0.5, SIDE_M * H3 / 3.0),
            _ => (SIDE_M * 0.5, SIDE_M * 0.5),
        };
        (
            self.ox + a * self.ux + b * self.vx,
            self.oz + a * self.uz + b * self.vz,
        )
    }

    /// Whether the row's footprint ([`Self::footprint`]) holds the world
    /// point, or comes within `tol` of it. A row with no footprint holds
    /// nothing.
    pub fn foot_contains(&self, x: f32, z: f32, tol: f32) -> bool {
        let (pts, n) = self.footprint();
        if n < 3 {
            return false;
        }
        // Which way round the corners go: a frame turned off the grid may
        // be mirrored, so the inside's side is read off the footprint.
        let (ax, az) = pts[0];
        let (bx, bz) = pts[1];
        let (cx, cz) = pts[2];
        let turn = if (bx - ax) * (cz - az) - (bz - az) * (cx - ax) >= 0.0 {
            1.0
        } else {
            -1.0
        };
        (0..n).all(|k| {
            let (ax, az) = pts[k];
            let (bx, bz) = pts[(k + 1) % n];
            let (ex, ez) = (bx - ax, bz - az);
            let len = (ex * ex + ez * ez).sqrt();
            turn * (ex * (z - az) - ez * (x - ax)) >= -tol * len
        })
    }

    /// The row's footprint, world XZ corners in order round it: a body's
    /// square or triangle, an edge's strip half a wall's thickness either
    /// side of its line and past its ends. The count is live; 0 for a
    /// flight on a triangle, whose turned frame does not say which third of
    /// the turn its triangle lies in, and for the legacy diagonal.
    pub fn footprint(&self) -> ([(f32, f32); 4], usize) {
        let at = |a: f32, b: f32| {
            (
                self.ox + a * self.ux + b * self.vx,
                self.oz + a * self.uz + b * self.vz,
            )
        };
        let t = WALL_THICKNESS_M * 0.5;
        match self.kind {
            HULL_TRI => (
                [
                    at(0.0, 0.0),
                    at(SIDE_M, 0.0),
                    at(SIDE_M * 0.5, SIDE_M * H3),
                    (0.0, 0.0),
                ],
                3,
            ),
            HULL_EDGE => (
                [at(-t, -t), at(SIDE_M + t, -t), at(SIDE_M + t, t), at(-t, t)],
                4,
            ),
            HULL_RISER if crate::support::foot_of(self.shape) == crate::sock::FOOT_TRI_RISER => {
                ([(0.0, 0.0); 4], 0)
            }
            HULL_PLANE | HULL_HALF | HULL_RISER => (
                [
                    at(0.0, 0.0),
                    at(SIDE_M, 0.0),
                    at(SIDE_M, SIDE_M),
                    at(0.0, SIDE_M),
                ],
                4,
            ),
            _ => ([(0.0, 0.0); 4], 0),
        }
    }

    /// Edge or diagonal against a mover (x, z) → (nx, nz) with an extra
    /// radius: where along the edge it meets, if it does — `collide.rs`'s
    /// `edge_meet` and `diag_meet`, in the hull's frame.
    fn meet(&self, x: f32, z: f32, nx: f32, nz: f32, r_extra: f32) -> Option<f32> {
        let (d0, d1, t0, along) = if self.kind == HULL_DIAG {
            let (x0, z0) = (self.ox, self.oz);
            if self.loc == LOC_DIAG_B {
                let d = |px: f32, pz: f32| ((px - x0) + (pz - z0) - SIDE_M) * INV_SQRT2;
                let s = |px: f32, pz: f32| ((px - x0) - (pz - z0) + SIDE_M) * INV_SQRT2;
                (d(x, z), d(nx, nz), s(x, z), s(nx, nz))
            } else {
                let d = |px: f32, pz: f32| ((pz - z0) - (px - x0)) * INV_SQRT2;
                let s = |px: f32, pz: f32| ((px - x0) + (pz - z0)) * INV_SQRT2;
                (d(x, z), d(nx, nz), s(x, z), s(nx, nz))
            }
        } else {
            let (t0, d0) = self.local(x, z);
            let (t, d1) = self.local(nx, nz);
            (d0, d1, t0, t)
        };
        let r = WALL_THICKNESS_M * 0.5 + r_extra;
        let crosses = (d0 < 0.0) != (d1 < 0.0);
        let pushes_in = fabs(d1) < r && fabs(d1) < fabs(d0);
        if !crosses && !pushes_in {
            return None;
        }
        // A crossing more than the mover's reach past an end went by the
        // wall, not through it. A body's step is shorter than that reach,
        // so this never decides one; it is what keeps a long probe (an
        // agent's route test across a metre) from meeting a wall whose
        // line it crossed beyond the wall's end.
        if crosses {
            let tc = t0 + (along - t0) * (d0 / (d0 - d1));
            if tc < -r || tc > self.span() + r {
                return None;
            }
        }
        Some(along)
    }

    fn span(&self) -> f32 {
        if self.kind == HULL_DIAG {
            DIAG_LEN_M
        } else {
            SIDE_M
        }
    }
}

/// Every built piece as a [`Hull`], filed by world bucket.
pub struct Hulls {
    /// Reserved whole at construction and never grown — `add` refuses at
    /// `MAX_PIECES` — so the tick allocates nothing, and an empty index
    /// costs no writes to build (tests build one per movement step).
    rows: Vec<Hull>,
    /// Key → row, open addressing like `build::AddrIndex`.
    keys: Box<[u64; PIECE_ADDR_SLOTS]>,
    vals: Box<[u16; PIECE_ADDR_SLOTS]>,
    space: Space,
}

impl Default for Hulls {
    fn default() -> Self {
        Self::new()
    }
}

/// The buckets a world box covers, clamped to the island.
#[inline]
fn bucket_range(x0: f32, z0: f32, x1: f32, z1: f32) -> (i32, i32, i32, i32) {
    let max = SPACE_SIDE as i32 - 1;
    let b = |v: f32| crate::fmath::floor_i32(v / SPACE_BUCKET_M).clamp(0, max);
    (b(x0), b(z0), b(x1), b(z1))
}

impl Hulls {
    pub fn new() -> Self {
        Self {
            rows: Vec::with_capacity(MAX_PIECES),
            keys: crate::boxed_array(0),
            vals: crate::boxed_array(0),
            space: Space::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn rows(&self) -> &[Hull] {
        &self.rows
    }

    pub fn clear(&mut self) {
        self.rows.clear();
        self.keys.fill(0);
        self.vals.fill(0);
        self.space.clear();
    }

    #[inline]
    fn home(key: u64) -> usize {
        (key.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> (64 - PIECE_ADDR_SLOTS.trailing_zeros()))
            as usize
            & (PIECE_ADDR_SLOTS - 1)
    }

    /// The row holding `key`.
    pub fn find(&self, key: u64) -> Option<&Hull> {
        self.index_of(key).map(|i| &self.rows[i])
    }

    fn index_of(&self, key: u64) -> Option<usize> {
        let mut i = Self::home(key);
        loop {
            let k = self.keys[i];
            if k == key {
                return Some(self.vals[i] as usize);
            }
            if k == 0 {
                return None;
            }
            i = (i + 1) & (PIECE_ADDR_SLOTS - 1);
        }
    }

    fn put_key(&mut self, key: u64, idx: usize) {
        let mut i = Self::home(key);
        while self.keys[i] != 0 && self.keys[i] != key {
            i = (i + 1) & (PIECE_ADDR_SLOTS - 1);
        }
        self.keys[i] = key;
        self.vals[i] = idx as u16;
    }

    fn del_key(&mut self, key: u64) {
        let mut i = Self::home(key);
        loop {
            let k = self.keys[i];
            if k == 0 {
                return;
            }
            if k == key {
                break;
            }
            i = (i + 1) & (PIECE_ADDR_SLOTS - 1);
        }
        self.keys[i] = 0;
        let mut j = i;
        loop {
            j = (j + 1) & (PIECE_ADDR_SLOTS - 1);
            let k = self.keys[j];
            if k == 0 {
                return;
            }
            let h = Self::home(k);
            if j.wrapping_sub(h) & (PIECE_ADDR_SLOTS - 1)
                >= j.wrapping_sub(i) & (PIECE_ADDR_SLOTS - 1)
            {
                self.keys[i] = k;
                self.vals[i] = self.vals[j];
                self.keys[j] = 0;
                i = j;
            }
        }
    }

    fn file(&mut self, i: usize) -> bool {
        let (x0, z0, x1, z1) = self.rows[i].aabb();
        let (bx0, bz0, bx1, bz1) = bucket_range(x0, z0, x1, z1);
        let n = ((bx1 - bx0 + 1) * (bz1 - bz0 + 1)) as usize;
        if self.space.len() + n > crate::limits::SPACE_SLOTS / 2 {
            return false;
        }
        for bx in bx0..=bx1 {
            for bz in bz0..=bz1 {
                self.space.add(bx as u16, bz as u16, i as u32);
            }
        }
        true
    }

    fn unfile(&mut self, i: usize) {
        let (x0, z0, x1, z1) = self.rows[i].aabb();
        let (bx0, bz0, bx1, bz1) = bucket_range(x0, z0, x1, z1);
        for bx in bx0..=bx1 {
            for bz in bz0..=bz1 {
                self.space.del(bx as u16, bz as u16, i as u32);
            }
        }
    }

    /// Add a row, or replace the one with its key. False when full — the
    /// caller refuses the placement it was filing.
    pub fn add(&mut self, h: Hull) -> bool {
        debug_assert!(h.key != 0);
        if let Some(i) = self.index_of(h.key) {
            self.unfile(i);
            self.rows[i] = h;
            return self.file(i);
        }
        if self.rows.len() >= MAX_PIECES {
            return false;
        }
        let i = self.rows.len();
        self.rows.push(h);
        if !self.file(i) {
            self.rows.pop();
            return false;
        }
        self.put_key(h.key, i);
        true
    }

    /// Take a row out; absent is a no-op. The last row moves into the hole
    /// and is re-filed under its new index.
    pub fn del(&mut self, key: u64) {
        let Some(i) = self.index_of(key) else {
            return;
        };
        self.unfile(i);
        self.del_key(key);
        let last = self.rows.len() - 1;
        if i != last {
            self.unfile(last);
        }
        self.rows.swap_remove(i);
        if i != last {
            self.file(i);
            self.put_key(self.rows[i].key, i);
        }
    }

    /// Close (or open) the opening of the row with `key`; `panes` marks a
    /// closed insert as glass or shutter. Opening clears both, closing
    /// without `panes` keeps what the row had — the column index's bits.
    pub fn set_shut(&mut self, key: u64, shut: bool, panes: bool) {
        if let Some(i) = self.index_of(key) {
            self.rows[i].shut = shut;
            self.rows[i].panes = shut && (panes || self.rows[i].panes);
        }
    }

    /// Visit every row whose filed box meets the world box, possibly more
    /// than once; stops when `f` answers true, and answers whether it did.
    #[inline]
    pub(crate) fn near(
        &self,
        x0: f32,
        z0: f32,
        x1: f32,
        z1: f32,
        mut f: impl FnMut(&Hull) -> bool,
    ) -> bool {
        if self.rows.is_empty() {
            return false;
        }
        let (bx0, bz0, bx1, bz1) = bucket_range(x0, z0, x1, z1);
        for bx in bx0..=bx1 {
            for bz in bz0..=bz1 {
                for i in self.space.at(bx as u16, bz as u16) {
                    if f(&self.rows[i as usize]) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// The highest standable top under the point (`x`, `z`) within a step
    /// of `feet_y`: bodies over their own footprint, flights at their
    /// ramp. `collide::piece_ground`'s piece half.
    pub fn ground<Y: Fn(&Hull) -> (f32, f32)>(&self, y: &Y, x: f32, z: f32, feet_y: f32) -> f32 {
        let lid = feet_y + STEP_UP;
        let mut best = NO_SURFACE;
        self.near(x, z, x, z, |h| {
            let top = match h.kind {
                HULL_PLANE | HULL_HALF | HULL_RISER | HULL_TRI => {
                    let (lx, lz) = h.local(x, z);
                    if !h.holds(lx, lz) {
                        return false;
                    }
                    if h.kind == HULL_RISER {
                        match crate::circulation::surface(h.shape, h.loc, lx, lz) {
                            Some(dy) => y(h).0 + dy,
                            None => return false,
                        }
                    } else if h.body_solid(lx, lz, 0.0) {
                        y(h).0
                    } else {
                        return false;
                    }
                }
                _ => return false,
            };
            if top <= lid && top > best {
                best = top;
            }
            false
        });
        best
    }

    /// Every body and flight whose footprint a capsule of radius `r` at
    /// (`x`, `z`) overlaps, as (slab level, top): `collide::any_body_plane`.
    /// Stops when `visit` answers true.
    pub fn bodies_near<Y: Fn(&Hull) -> (f32, f32)>(
        &self,
        y: &Y,
        x: f32,
        z: f32,
        r: f32,
        mut visit: impl FnMut(usize, f32) -> bool,
    ) -> bool {
        self.near(x - r, z - r, x + r, z + r, |h| {
            if !h.is_body() {
                return false;
            }
            let (lx, lz) = h.local(x, z);
            if h.foot_dist2(lx, lz) >= r * r {
                return false;
            }
            if h.kind == HULL_RISER {
                let Some(dy) = crate::circulation::surface(h.shape, h.loc, lx, lz) else {
                    return false;
                };
                let slab = if h.shape == SHAPE_FOUNDATION_STEPS {
                    0
                } else {
                    (h.level as usize).max(1)
                };
                return visit(slab, y(h).0 + dy);
            }
            let here = match h.kind {
                HULL_PLANE => !h.open || floor_frame_solid(lx, lz, r),
                // Its true footprint at the radius, the rim too.
                HULL_TRI => !h.open || tri_rim_solid(lx, lz, r),
                // The half test at the point, boundary-inclusive both sides
                // (a NW+SE seam reads the same from either), the rim at
                // the radius.
                _ => {
                    let inside = match h.loc {
                        LOC_TRI_XLO_ZLO => lx + lz <= SIDE_M,
                        LOC_TRI_XHI_ZHI => lx + lz >= SIDE_M,
                        LOC_TRI_XHI_ZLO => lz <= lx,
                        _ => lz >= lx,
                    };
                    inside && (!h.open || tri_frame_solid(h.loc, lx, lz, r))
                }
            };
            here && visit(h.level as usize, y(h).0)
        })
    }

    /// The floor a body standing at (`x`, `z`) with its feet at `feet_y`
    /// is on: `collide::floor_under`'s piece walk, the lowest socket at
    /// the first of its five probe points that finds one.
    pub fn floor_under<Y: Fn(&Hull) -> (f32, f32)>(
        &self,
        yf: &Y,
        x: f32,
        z: f32,
        feet_y: f32,
    ) -> Option<(u16, u16, u8)> {
        const HAND: f32 = 0.25;
        let r = CAPSULE_RADIUS_M;
        for (px, pz) in [(x, z), (x - r, z), (x + r, z), (x, z - r), (x, z + r)] {
            let mut best: Option<(u8, PieceHit)> = None;
            self.near(px, pz, px, pz, |h| {
                if !h.is_body() {
                    return false;
                }
                let (lx, lz) = h.local(px, pz);
                if !h.holds(lx, lz) {
                    return false;
                }
                let y = yf(h).0;
                let on = if h.kind == HULL_RISER {
                    feet_y > y - HAND && feet_y < y + LEVEL_H_M + HAND
                } else {
                    fabs(y - feet_y) <= HAND
                };
                if on && best.is_none_or(|(l, _)| h.level < l) {
                    best = Some((h.level, h.hit));
                }
                false
            });
            if let Some((level, hit)) = best {
                return Some((hit.cx, hit.cz, level));
            }
        }
        None
    }

    /// Whether an edge or diagonal stops a capsule moving (x, z) →
    /// (nx, nz) with its feet at `feet_y`: `collide::blocked`'s piece walk.
    pub fn edges_block<Y: Fn(&Hull) -> (f32, f32)>(
        &self,
        y: &Y,
        x: f32,
        z: f32,
        nx: f32,
        nz: f32,
        feet_y: f32,
    ) -> bool {
        let reach = CAPSULE_RADIUS_M + WALL_THICKNESS_M * 0.5;
        let (lo, hi) = (feet_y, feet_y + CAPSULE_HEIGHT_M);
        self.near(
            x.min(nx) - reach,
            z.min(nz) - reach,
            x.max(nx) + reach,
            z.max(nz) + reach,
            |h| match h.kind {
                HULL_EDGE => {
                    // The plan test first: it is arithmetic, the heights
                    // are lookups.
                    let Some(t) = h.meet(x, z, nx, nz, CAPSULE_RADIUS_M) else {
                        return false;
                    };
                    let (bottom, foot) = y(h);
                    if feet_y >= bottom + h.height() || feet_y + CAPSULE_HEIGHT_M <= foot {
                        return false;
                    }
                    let full = (0.0..=SIDE_M).contains(&t);
                    if (feet_y < bottom && foot < bottom) || h.shut {
                        return full;
                    }
                    match h.shape {
                        SHAPE_DOORWAY => doorway_solid_at(t),
                        SHAPE_FRAME => {
                            (0.0..=FRAME_RIM_M).contains(&t)
                                || (SIDE_M - FRAME_RIM_M..=SIDE_M).contains(&t)
                        }
                        _ => full,
                    }
                }
                HULL_DIAG => {
                    if !h
                        .meet(x, z, nx, nz, CAPSULE_RADIUS_M)
                        .is_some_and(|t| (0.0..=h.span()).contains(&t))
                    {
                        return false;
                    }
                    let bottom = y(h).0;
                    let top = bottom + h.height();
                    lo < top && hi > bottom
                }
                _ => false,
            },
        )
    }

    /// Which edge or diagonal a shot sample moving (x, z) → (nx, nz) at
    /// altitude `y` with radius `r` stops on, and whether it was the insert
    /// in it: `collide::shot_hit`'s edge and diagonal walks. Among several,
    /// the least [`Hull::rank`].
    #[allow(clippy::too_many_arguments)]
    pub fn shot_edges<Y: Fn(&Hull) -> (f32, f32)>(
        &self,
        yf: &Y,
        x: f32,
        z: f32,
        nx: f32,
        nz: f32,
        y: f32,
        r: f32,
    ) -> Option<(Hull, bool)> {
        let reach = r + WALL_THICKNESS_M * 0.5;
        let mut best: Option<(Hull, bool, ShotOrder)> = None;
        let cells = (
            crate::build::build_cell_of(nx),
            crate::build::build_cell_of(nz),
            crate::build::build_cell_of(x),
            crate::build::build_cell_of(z),
        );
        self.near(
            x.min(nx) - reach,
            z.min(nz) - reach,
            x.max(nx) + reach,
            z.max(nz) + reach,
            |h| {
                let got = match h.kind {
                    HULL_EDGE => h
                        .meet(x, z, nx, nz, r)
                        .and_then(|t| shot_edge(h, yf(h), t, y, r)),
                    HULL_DIAG => h
                        .meet(x, z, nx, nz, r)
                        .filter(|t| (0.0..=h.span()).contains(t))
                        .and_then(|_| {
                            let bottom = yf(h).0;
                            (y >= bottom && y < bottom + h.height()).then_some(false)
                        }),
                    _ => None,
                };
                if let Some(insert) = got {
                    let order = shot_order(h, cells, (x, z, nx, nz));
                    if best.is_none_or(|(_, _, o)| order < o) {
                        best = Some((*h, insert, order));
                    }
                }
                false
            },
        );
        best.map(|(h, insert, _)| (h, insert))
    }

    /// Which body or flight a shot sample at (`x`, `z`), altitude `y`,
    /// radius `r` stops on: `collide::cell_planes_stop_shot`. The least
    /// [`Hull::rank`] among several.
    pub fn shot_bodies<Y: Fn(&Hull) -> (f32, f32)>(
        &self,
        yf: &Y,
        x: f32,
        z: f32,
        y: f32,
        r: f32,
    ) -> Option<Hull> {
        let mut best: Option<Hull> = None;
        self.near(x - r, z - r, x + r, z + r, |h| {
            if !h.is_body() {
                return false;
            }
            let (lx, lz) = h.local(x, z);
            if h.foot_dist2(lx, lz) >= r * r {
                return false;
            }
            let stops = if h.kind == HULL_RISER {
                crate::circulation::surface(h.shape, h.loc, lx, lz).is_some_and(|dy| {
                    let top = yf(h).0 + dy;
                    y - r < top
                        && (h.shape == SHAPE_FOUNDATION_STEPS || y + r > top - PLANE_THICKNESS_M)
                })
            } else {
                let here = match h.kind {
                    HULL_PLANE => !h.open || floor_frame_solid(lx, lz, r),
                    HULL_TRI => !h.open || tri_rim_solid(lx, lz, r),
                    _ => {
                        let inside = match h.loc {
                            LOC_TRI_XLO_ZLO => lx + lz <= SIDE_M,
                            LOC_TRI_XHI_ZHI => lx + lz >= SIDE_M,
                            LOC_TRI_XHI_ZLO => lz <= lx,
                            _ => lz >= lx,
                        };
                        inside && (!h.open || tri_frame_solid(h.loc, lx, lz, r))
                    }
                };
                let floor = yf(h).0;
                here && y - r < floor && (h.level == 0 || y + r > floor - PLANE_THICKNESS_M)
            };
            if stops && best.is_none_or(|b| h.rank() < b.rank()) {
                best = Some(*h);
            }
            false
        });
        best
    }

    /// The floor of a slab the grid cannot name (freeform W4) at socket
    /// `level` whose footprint holds (`x`, `z`) — a square or a triangle,
    /// never a frame or a flight — as the row stores it; the highest, should
    /// two buildings' slabs overlap there, so the answer is the rows' and
    /// not their filing order's. What a deployable on a triangle foundation,
    /// or on a floor of a building turned off the grid, stands on
    /// (`deploy::built_floor`).
    pub fn posed_floor(&self, level: u8, x: f32, z: f32) -> Option<f32> {
        let mut best: Option<f32> = None;
        self.near(x, z, x, z, |h| {
            if h.hit.loc == crate::build::LOC_NONE && h.level == level && h.slab_holds(x, z) {
                best = Some(best.map_or(h.floor_y, |b| b.max(h.floor_y)));
            }
            false
        });
        best
    }

    /// The top of the slab at socket `level` under (`x`, `z`) — any whole
    /// body, the grid's or not, never a frame — and whether it is one the
    /// grid cannot name; the highest, as [`Self::posed_floor`] picks.
    pub fn slab_top(&self, level: u8, x: f32, z: f32) -> Option<(f32, bool)> {
        let mut best: Option<(f32, bool)> = None;
        self.near(x, z, x, z, |h| {
            if h.level != level || h.open || !matches!(h.kind, HULL_PLANE | HULL_HALF | HULL_TRI) {
                return false;
            }
            let (lx, lz) = h.local(x, z);
            if h.holds(lx, lz)
                && h.body_solid(lx, lz, 0.0)
                && best.is_none_or(|(y, _)| h.floor_y > y)
            {
                best = Some((h.floor_y, h.hit.loc == crate::build::LOC_NONE));
            }
            false
        });
        best
    }

    /// The first wall the grid cannot name (freeform) a step `a` → `b`
    /// crosses — its line, between its ends and its floor and head — as
    /// (how far along the step, the row). The build aim's posed half of
    /// meeting a wall.
    pub fn posed_edge_crossed(&self, a: [f32; 3], b: [f32; 3]) -> Option<(f32, Hull)> {
        let mut best: Option<(f32, Hull)> = None;
        let (x0, z0) = (a[0].min(b[0]), a[2].min(b[2]));
        let (x1, z1) = (a[0].max(b[0]), a[2].max(b[2]));
        self.near(x0, z0, x1, z1, |h| {
            if h.hit.loc != crate::build::LOC_NONE || h.kind != HULL_EDGE {
                return false;
            }
            let (t0, d0) = h.local(a[0], a[2]);
            let (t1, d1) = h.local(b[0], b[2]);
            if (d0 < 0.0) == (d1 < 0.0) || d0 == d1 {
                return false;
            }
            let s = d0 / (d0 - d1);
            let t = t0 + (t1 - t0) * s;
            let y = a[1] + (b[1] - a[1]) * s;
            if !(0.0..=SIDE_M).contains(&t) || y < h.floor_y || y > h.floor_y + h.height() {
                return false;
            }
            if best.is_none_or(|(bs, _)| s < bs) {
                best = Some((s, *h));
            }
            false
        });
        best
    }

    /// The upper storey of a slab the grid cannot name a step `a` → `b`
    /// crosses the plane of just past its footprint — within `band` of it,
    /// over no slab of that storey — as (its socket, how far along the
    /// step): the build aim's posed half of continuing a floor out past its
    /// edge.
    pub fn posed_socket_crossed(&self, a: [f32; 3], b: [f32; 3], band: f32) -> Option<(u8, f32)> {
        if a[1] == b[1] {
            return None;
        }
        let mut best: Option<(u8, f32)> = None;
        let (x0, z0) = (a[0].min(b[0]) - band, a[2].min(b[2]) - band);
        let (x1, z1) = (a[0].max(b[0]) + band, a[2].max(b[2]) + band);
        self.near(x0, z0, x1, z1, |h| {
            if h.hit.loc != crate::build::LOC_NONE
                || h.level == 0
                || !matches!(h.kind, HULL_PLANE | HULL_TRI)
            {
                return false;
            }
            let s = (h.floor_y - a[1]) / (b[1] - a[1]);
            if !(0.0..=1.0).contains(&s) || best.is_some_and(|(_, bs)| bs <= s) {
                return false;
            }
            let (x, z) = (a[0] + (b[0] - a[0]) * s, a[2] + (b[2] - a[2]) * s);
            let (lx, lz) = h.local(x, z);
            if h.holds(lx, lz) || h.foot_dist2(lx, lz) > band * band {
                return false;
            }
            best = Some((h.level, s));
            false
        });
        // Over a slab of that storey already, it is that slab's floor.
        let (level, s) = best?;
        let (x, z) = (a[0] + (b[0] - a[0]) * s, a[2] + (b[2] - a[2]) * s);
        let mut covered = false;
        self.near(x, z, x, z, |h| {
            if h.hit.loc == crate::build::LOC_NONE && h.level == level && h.is_body() {
                let (lx, lz) = h.local(x, z);
                covered |= h.holds(lx, lz);
            }
            covered
        });
        (!covered).then_some((level, s))
    }

    /// The socket of a slab the grid cannot name whose top at (`x`, `z`)
    /// is `y`: the storey an aim that met that surface is on, read off the
    /// slab rather than off the column under it, which a building turned
    /// off the grid need not share a band with.
    pub fn posed_level(&self, x: f32, z: f32, y: f32) -> Option<u8> {
        let mut best: Option<u8> = None;
        self.near(x, z, x, z, |h| {
            if h.hit.loc == crate::build::LOC_NONE
                && fabs(h.floor_y - y) < 0.01
                && h.slab_holds(x, z)
            {
                best = Some(best.map_or(h.level, |b| b.min(h.level)));
            }
            false
        });
        best
    }
}

impl Hulls {
    /// Whether a body's flank stops a capsule standing at (`x`, `z`) with
    /// its feet at `feet_y`: `collide::plane_blocked`'s piece half.
    pub fn plane_blocked<Y: Fn(&Hull) -> (f32, f32)>(
        &self,
        y: &Y,
        x: f32,
        z: f32,
        feet_y: f32,
    ) -> bool {
        let head = feet_y + CAPSULE_HEIGHT_M;
        self.bodies_near(y, x, z, CAPSULE_RADIUS_M, |level, top| {
            feet_y + STEP_UP < top && (level == 0 || head > top - PLANE_THICKNESS_M)
        })
    }

    /// The lowest slab underside over a capsule's head: `collide::piece_ceiling`.
    pub fn ceiling<Y: Fn(&Hull) -> (f32, f32)>(
        &self,
        y: &Y,
        x: f32,
        z: f32,
        feet_y: f32,
    ) -> Option<f32> {
        let head = feet_y + CAPSULE_HEIGHT_M;
        let mut lowest: Option<f32> = None;
        self.bodies_near(y, x, z, CAPSULE_RADIUS_M, |level, top| {
            let underside = top - PLANE_THICKNESS_M;
            if level > 0 && head <= underside {
                lowest = Some(lowest.map_or(underside, |old| old.min(underside)));
            }
            false
        });
        lowest
    }

    /// The highest top within a step of `feet_y` a capsule's disc overlaps:
    /// `collide::body_ground`'s piece half.
    pub fn body_ground<Y: Fn(&Hull) -> (f32, f32)>(
        &self,
        y: &Y,
        x: f32,
        z: f32,
        feet_y: f32,
    ) -> f32 {
        let lid = feet_y + STEP_UP;
        let mut best = NO_SURFACE;
        self.bodies_near(y, x, z, CAPSULE_RADIUS_M, |_, top| {
            if top <= lid && top > best {
                best = top;
            }
            false
        });
        best
    }

    /// What a shot sample moving (x, z) → (nx, nz) at altitude `y` with
    /// radius `r` stops on, and whether it was an insert: edges and
    /// diagonals first, then the bodies at the new point.
    #[allow(clippy::too_many_arguments)]
    pub fn shot_hit<Y: Fn(&Hull) -> (f32, f32)>(
        &self,
        yf: &Y,
        x: f32,
        z: f32,
        nx: f32,
        nz: f32,
        y: f32,
        r: f32,
    ) -> Option<(Hull, bool)> {
        self.shot_edges(yf, x, z, nx, nz, y, r)
            .or_else(|| self.shot_bodies(yf, nx, nz, y, r).map(|h| (h, false)))
    }
}

/// The order a shot sample prefers among the edges and diagonals it stops
/// on: the order the column index walked them in for a grid row — the
/// destination cell's four edges, then the origin's, levels up within each,
/// then the diagonals the same way — and for any other row the point along
/// the sample where it crosses the row's line, earliest first. Ties fall to
/// [`Hull::rank`].
type ShotOrder = (u32, u8, u8, (u8, u16, u16, u8, u8));

fn shot_order(h: &Hull, cells: (i32, i32, i32, i32), seg: (f32, f32, f32, f32)) -> ShotOrder {
    use crate::build::LOC_DIAG_A;
    let (bx1, bz1, bx0, bz0) = cells;
    let (cx, cz) = (h.hit.cx as i32, h.hit.cz as i32);
    let edge_at = |bx: i32, bz: i32| -> Option<u32> {
        match h.hit.loc {
            LOC_EDGE_XLO if cz == bz && cx == bx => Some(0),
            LOC_EDGE_XLO if cz == bz && cx == bx + 1 => Some(1),
            LOC_EDGE_ZLO if cx == bx && cz == bz => Some(2),
            LOC_EDGE_ZLO if cx == bx && cz == bz + 1 => Some(3),
            _ => None,
        }
    };
    let moved = (bx0, bz0) != (bx1, bz1);
    let at = match (h.kind, h.hit.loc) {
        (HULL_EDGE, LOC_EDGE_XLO | LOC_EDGE_ZLO) => edge_at(bx1, bz1)
            .or_else(|| edge_at(bx0, bz0).filter(|_| moved).map(|i| i + 4))
            .unwrap_or(8),
        (HULL_DIAG, _) => {
            if (cx, cz) == (bx1, bz1) {
                10
            } else if moved && (cx, cz) == (bx0, bz0) {
                11
            } else {
                12
            }
        }
        _ => {
            // Where along the sample it crosses the row's line.
            let (x, z, nx, nz) = seg;
            let (_, d0) = h.local(x, z);
            let (_, d1) = h.local(nx, nz);
            let f = if (d0 < 0.0) != (d1 < 0.0) {
                d0 / (d0 - d1)
            } else {
                1.0
            };
            20 + (f.clamp(0.0, 1.0) * 1024.0) as u32
        }
    };
    let sub = (h.kind == HULL_DIAG && h.hit.loc != LOC_DIAG_A) as u8;
    (at, h.level, sub, h.rank())
}

/// A straight edge a shot sample meets `t` along: `collide::cell_edges_stop_shot`'s
/// per-level body. Some(insert) when it stops the shot.
fn shot_edge(h: &Hull, (bottom, foot): (f32, f32), t: f32, y: f32, r: f32) -> Option<bool> {
    if y < foot || y >= bottom + h.height() {
        return None;
    }
    let span = (0.0..=SIDE_M).contains(&t);
    let solid = span
        && (y < bottom
            || match h.shape {
                SHAPE_DOORWAY => doorway_solid_at(t) || y - bottom >= DOOR_HEAD_M,
                SHAPE_WINDOW => window_solid_at(t, y - bottom),
                SHAPE_FRAME => frame_solid_at(t, y - bottom),
                _ => true,
            });
    let arch = match h.shape {
        SHAPE_DOORWAY => crate::deploy::ARCH_DOOR,
        SHAPE_WINDOW if h.panes => crate::deploy::ARCH_WINDOW_GLASS,
        SHAPE_WINDOW => crate::deploy::ARCH_WINDOW_BARS,
        _ => crate::deploy::ARCH_GARAGE_DOOR,
    };
    let insert = !solid && h.shut && crate::deploy::insert_solid_at(arch, t, y - bottom, r);
    (solid || insert).then_some(insert)
}

/// The hull of a piece at its place in a building: the footprint's own
/// frame carried into the world by the building's (`sock::Sock::local_axes`,
/// `Frame::to_world`), its floor the building's band plus the socket's
/// height. A grid piece in a grid building comes out as [`grid_hull`]
/// does, to the bit (`tests::a_posed_grid_piece_is_its_grid_hull`).
pub fn posed_hull(
    key: u64,
    hit: PieceHit,
    frame: &crate::frame::Frame,
    addr: &crate::sock::PieceAddr,
    shape: u8,
) -> Hull {
    use crate::sock::{FOOT_EDGE, FOOT_RISER, FOOT_SQUARE, FOOT_TRI};
    let (o, ux, uz) = addr.sock.local_axes();
    let (ox, oz) = frame.to_world(o);
    let (uxx, uxz) = frame.axis(ux);
    let (vx, vz) = frame.axis(uz);
    let kind = match addr.sock.foot {
        FOOT_SQUARE => HULL_PLANE,
        FOOT_TRI => HULL_TRI,
        FOOT_EDGE => HULL_EDGE,
        FOOT_RISER | crate::sock::FOOT_TRI_RISER => HULL_RISER,
        _ => HULL_TRI,
    };
    let floor_y = crate::build::band_y(frame.band) + crate::build::level_y(addr.level);
    Hull {
        key,
        hit,
        kind,
        shape,
        // A flight's frame is already turned to climb +z.
        loc: if kind == HULL_RISER {
            crate::build::LOC_RISER
        } else {
            0
        },
        level: addr.level,
        open: matches!(
            shape,
            crate::build::SHAPE_FLOOR_FRAME | crate::build::SHAPE_TRI_FLOOR_FRAME
        ),
        shut: false,
        panes: false,
        floor_y,
        foot_y: floor_y,
        ox,
        oz,
        ux: uxx,
        uz: uxz,
        vx,
        vz,
    }
}

/// The row key of a grid address — `build`'s address key.
pub const fn grid_key(cx: u16, cz: u16, level: u8, loc: u8) -> u64 {
    (1 << 63) | (cx as u64) << 32 | (cz as u64) << 16 | (level as u64) << 8 | loc as u64
}

/// Heights as the row holds them.
pub fn stored(h: &Hull) -> (f32, f32) {
    (h.floor_y, h.foot_y)
}

/// The hull of a grid piece: its cell's frame, and the legacy kind its
/// address names. `floor_y` and `foot_y` are the caller's — the column's
/// floor at the piece's level, and how far an edge reaches below it.
#[allow(clippy::too_many_arguments)]
pub fn grid_hull(
    key: u64,
    cx: u16,
    cz: u16,
    level: u8,
    loc: u8,
    shape: u8,
    floor_y: f32,
    foot_y: f32,
) -> Hull {
    let x0 = cx as f32 * SIDE_M;
    let z0 = cz as f32 * SIDE_M;
    let kind = match loc {
        LOC_PLANE => HULL_PLANE,
        LOC_EDGE_XLO | LOC_EDGE_ZLO => HULL_EDGE,
        LOC_TRI_XLO_ZLO | LOC_TRI_XHI_ZLO | LOC_TRI_XLO_ZHI | LOC_TRI_XHI_ZHI => HULL_HALF,
        crate::build::LOC_DIAG_A | LOC_DIAG_B => HULL_DIAG,
        _ => HULL_RISER,
    };
    // An x-low edge runs along +z with the cell (+x) on its normal's side;
    // everything else is the cell's own frame.
    let (ux, uz, vx, vz) = if loc == LOC_EDGE_XLO {
        (0.0, 1.0, 1.0, 0.0)
    } else {
        (1.0, 0.0, 0.0, 1.0)
    };
    Hull {
        key,
        hit: PieceHit { cx, cz, level, loc },
        kind,
        shape,
        loc,
        level,
        open: matches!(
            shape,
            crate::build::SHAPE_FLOOR_FRAME | crate::build::SHAPE_TRI_FLOOR_FRAME
        ),
        shut: false,
        panes: false,
        floor_y,
        foot_y,
        ox: x0,
        oz: z0,
        ux,
        uz,
        vx,
        vz,
    }
}

#[cfg(test)]
mod tests {
    use super::stored as STORED;
    use super::*;
    use crate::build::{SHAPE_FOUNDATION, SHAPE_WALL};
    use crate::rng::Pcg32;

    fn key(n: u64) -> u64 {
        (1 << 63) | n
    }

    /// Adds, replaces and deletes against a plain list: every row is found
    /// by its key and by a query at its centre after every operation —
    /// the swap-remove re-file is what this is for.
    #[test]
    fn churn_keeps_every_row_findable() {
        let mut rng = Pcg32::new(0x4011, 3);
        let mut hulls = Hulls::new();
        let mut shadow: Vec<(u64, u16, u16)> = Vec::new();
        for _ in 0..4_000 {
            let n = rng.next_bounded(60) as u64;
            let (cx, cz) = (
                600 + rng.next_bounded(8) as u16,
                600 + rng.next_bounded(8) as u16,
            );
            if rng.next_bounded(3) != 0 {
                let h = grid_hull(key(n), cx, cz, 0, LOC_PLANE, SHAPE_FOUNDATION, 10.0, 10.0);
                assert!(hulls.add(h));
                shadow.retain(|e| e.0 != key(n));
                shadow.push((key(n), cx, cz));
            } else {
                hulls.del(key(n));
                shadow.retain(|e| e.0 != key(n));
            }
            assert_eq!(hulls.len(), shadow.len());
            for &(k, cx, cz) in &shadow {
                let h = hulls.find(k).expect("row by key");
                assert_eq!((h.hit.cx, h.hit.cz), (cx, cz));
                let (x, z) = (cx as f32 * 3.0 + 1.5, cz as f32 * 3.0 + 1.5);
                let mut seen = false;
                hulls.near(x, z, x, z, |r| {
                    seen |= r.key == k;
                    false
                });
                assert!(seen, "row {k:x} not filed at its centre");
            }
        }
        hulls.clear();
        assert!(hulls.is_empty());
    }

    /// A grid piece placed through a building's frame answers every query
    /// exactly as the cell-framed row does: frame ∘ lattice reproduces the
    /// grid, to the bit.
    #[test]
    fn a_posed_grid_piece_is_its_grid_hull() {
        use crate::build::*;
        use crate::frame::Frame;
        use crate::sock::{from_grid, PieceAddr};
        let (ox, oz) = (680u16, 690u16);
        let frame = Frame::new((ox as i32 * 300, oz as i32 * 300), 41, 0);
        let mut rng = Pcg32::new(0x9_05ED, 5);
        let cases = [
            (LOC_PLANE, SHAPE_FLOOR, 2u8),
            (LOC_PLANE, SHAPE_FLOOR_FRAME, 3),
            (LOC_PLANE, SHAPE_FOUNDATION, 0),
            (LOC_EDGE_XLO, SHAPE_WALL, 0),
            (LOC_EDGE_ZLO, SHAPE_DOORWAY, 2),
            (LOC_EDGE_XLO, SHAPE_WINDOW, 9),
            (LOC_EDGE_ZLO, SHAPE_FRAME, 1),
            (LOC_RISER, SHAPE_STAIRS, 0),
            (LOC_RISER_XHI, SHAPE_STAIRS_L, 2),
            (LOC_RISER_ZLO, SHAPE_STAIRS_U, 1),
            (LOC_RISER_XLO, SHAPE_STAIRS_SPIRAL, 4),
            (LOC_RISER_XHI, SHAPE_FOUNDATION_STEPS, 0),
        ];
        for (loc, shape, level) in cases {
            let (cx, cz) = (ox + 2, oz + 1);
            let sock = from_grid(ox, oz, cx, cz, loc).unwrap();
            let addr = PieceAddr {
                bld: 3,
                level,
                sock,
            };
            let floor = band_y(41) + level_y(level);
            let mut grid = Hulls::new();
            grid.add(grid_hull(key(1), cx, cz, level, loc, shape, floor, floor));
            let mut posed = Hulls::new();
            let h = posed_hull(key(1), grid.rows()[0].hit, &frame, &addr, shape);
            assert_eq!(h.floor_y.to_bits(), floor.to_bits());
            assert_eq!((h.ox, h.oz), {
                let g = grid.rows()[0];
                if loc == LOC_EDGE_XLO || loc == LOC_EDGE_ZLO || loc == LOC_PLANE {
                    (g.ox, g.oz)
                } else {
                    (h.ox, h.oz)
                }
            });
            posed.add(h);
            let (x0, z0) = (cx as f32 * 3.0 - 1.0, cz as f32 * 3.0 - 1.0);
            for i in 0..4_000 {
                let u = |rng: &mut Pcg32| rng.next_bounded(1 << 16) as f32 / 65536.0;
                let (x, z) = (x0 + u(&mut rng) * 5.0, z0 + u(&mut rng) * 5.0);
                let (nx, nz) = (x + (u(&mut rng) - 0.5) * 0.8, z + (u(&mut rng) - 0.5) * 0.8);
                let feet = floor + (u(&mut rng) - 0.5) * 6.0;
                let y = feet + u(&mut rng) * 2.0;
                let ctx = (loc, shape, i);
                assert_eq!(
                    grid.ground(&STORED, x, z, feet).to_bits(),
                    posed.ground(&STORED, x, z, feet).to_bits(),
                    "{ctx:?}"
                );
                assert_eq!(
                    grid.body_ground(&STORED, x, z, feet).to_bits(),
                    posed.body_ground(&STORED, x, z, feet).to_bits(),
                    "{ctx:?}"
                );
                assert_eq!(
                    grid.plane_blocked(&STORED, x, z, feet),
                    posed.plane_blocked(&STORED, x, z, feet),
                    "{ctx:?}"
                );
                assert_eq!(
                    grid.edges_block(&STORED, x, z, nx, nz, feet),
                    posed.edges_block(&STORED, x, z, nx, nz, feet),
                    "{ctx:?}"
                );
                for r in [0.0, 0.1] {
                    assert_eq!(
                        grid.shot_hit(&STORED, x, z, nx, nz, y, r).map(|h| h.1),
                        posed.shot_hit(&STORED, x, z, nx, nz, y, r).map(|h| h.1),
                        "{ctx:?}"
                    );
                }
            }
        }
    }

    /// An equilateral floor carries a body over its own triangle and not a
    /// hand's breadth past its sides, and blocks a flank the same.
    #[test]
    fn a_triangle_floor_is_its_triangle() {
        use crate::build::*;
        use crate::frame::Frame;
        use crate::lattice::LPt;
        use crate::sock::{PieceAddr, Sock};
        let frame = Frame::new((200_000, 200_000), 40, 7_000);
        let sock = Sock::tri_from(LPt::ZERO, 1);
        let addr = PieceAddr {
            bld: 0,
            level: 2,
            sock,
        };
        let mut hulls = Hulls::new();
        let h = posed_hull(key(9), PieceHit::default(), &frame, &addr, SHAPE_TRI_FLOOR);
        hulls.add(h);
        let floor = h.floor_y;
        // The centroid is inside, a point 0.1 m past a corner is not.
        let (cx, cz) = frame.to_world(sock.at);
        assert_eq!(
            hulls.ground(&STORED, cx, cz, floor).to_bits(),
            floor.to_bits()
        );
        let (corner, _) = sock.corners();
        let (kx, kz) = frame.to_world(corner[1]);
        let (dx, dz) = (kx - cx, kz - cz);
        let n = (dx * dx + dz * dz).sqrt();
        let (fx, fz) = (kx + dx / n * 0.1, kz + dz / n * 0.1);
        assert_eq!(hulls.ground(&STORED, fx, fz, floor), NO_SURFACE);
        // A body standing there still overlaps it with its disc.
        assert_eq!(hulls.body_ground(&STORED, fx, fz, floor), floor);
        // And a body 0.5 m past it does not.
        let (gx, gz) = (kx + dx / n * 0.5, kz + dz / n * 0.5);
        assert_eq!(hulls.body_ground(&STORED, gx, gz, floor), NO_SURFACE);
        // Its slab stops a body walking in below its underside but above a
        // step.
        assert!(hulls.plane_blocked(&STORED, cx, cz, floor - 1.0));
    }

    /// An agent's route test crosses a metre at a time. One that starts on
    /// the line a diagonal wall would run on past its end crosses that line
    /// beside the wall, not through it, and is not stopped — the case that
    /// stalled the survivor agents (`server/tests/agent_walls.rs`) when
    /// every wall near a move became a candidate, not just its two cells'.
    #[test]
    fn a_long_probe_past_a_walls_end_is_not_stopped() {
        use crate::build::{LOC_DIAG_B, SHAPE_WALL};
        let mut hulls = Hulls::new();
        // Cell (585, 172): its B diagonal runs (1758, 516) → (1755, 519).
        hulls.add(grid_hull(
            key(1),
            585,
            172,
            0,
            LOC_DIAG_B,
            SHAPE_WALL,
            2.0,
            2.0,
        ));
        assert!(!hulls.edges_block(&STORED, 1754.5, 519.5, 1754.5, 518.5, 2.38));
        // Walking into it still is.
        assert!(hulls.edges_block(&STORED, 1756.5, 518.5, 1756.2, 518.2, 2.38));
    }

    #[test]
    fn a_wall_blocks_a_crossing_and_not_a_pass_beside_it() {
        let mut hulls = Hulls::new();
        let (cx, cz) = (700u16, 700u16);
        hulls.add(grid_hull(
            key(1),
            cx,
            cz,
            0,
            LOC_EDGE_XLO,
            SHAPE_WALL,
            20.0,
            20.0,
        ));
        let x = cx as f32 * 3.0;
        let z = cz as f32 * 3.0 + 1.5;
        assert!(hulls.edges_block(&STORED, x - 0.6, z, x - 0.1, z, 20.0));
        assert!(!hulls.edges_block(&STORED, x - 0.6, z + 4.0, x + 0.6, z + 4.0, 20.0));
        assert!(
            !hulls.edges_block(&STORED, x - 0.6, z, x - 0.1, z, 23.5),
            "above it"
        );
    }
}

/// The hulls against the column index they replace: random grid bases of
/// every legacy shape, random bodies and shots, the same answers.
#[cfg(test)]
mod shadow {
    use super::stored as STORED;
    use super::*;
    use crate::build::*;
    use crate::collide::{self, ColIndex};
    use crate::limits::MAX_BUILD_SOCKETS;
    use crate::rng::Pcg32;

    const SEED: u64 = 20260731;
    const C0: u16 = 680;
    const N: u16 = 7;

    fn hv() -> &'static crate::terrain::Haven {
        static HV: std::sync::OnceLock<crate::terrain::Haven> = std::sync::OnceLock::new();
        HV.get_or_init(|| crate::terrain::haven(SEED))
    }

    struct Base {
        cols: ColIndex,
        hulls: Box<Hulls>,
    }

    fn key(cx: u16, cz: u16, level: u8, loc: u8) -> u64 {
        grid_key(cx, cz, level, loc)
    }

    /// One random base: at most one piece per (cell, level, slot), the
    /// occupancy every placement rule enforces, so each mask bit is one row.
    fn base(rng: &mut Pcg32) -> Base {
        let mut cols = ColIndex::new();
        let mut placed: Vec<(u16, u16, u8, u8, u8)> = Vec::new();
        let mut shut: Vec<(u16, u16, u8, u8, bool)> = Vec::new();
        let mut plates = [[0i8; N as usize + 1]; N as usize + 1];
        for row in plates.iter_mut() {
            for p in row.iter_mut() {
                *p = rng.next_bounded(7) as i8 - 3;
            }
        }
        let plate = |cx: u16, cz: u16| plates[(cx - C0) as usize][(cz - C0) as usize];
        let mut put = |cols: &mut ColIndex, cx: u16, cz: u16, level: u8, loc: u8, shape: u8| {
            cols.add(cx, cz, level, loc, shape, plate(cx, cz));
            placed.push((cx, cz, level, loc, shape));
        };
        let edge_shapes = [
            SHAPE_WALL,
            SHAPE_HALF_WALL,
            SHAPE_LOW_WALL,
            SHAPE_DOORWAY,
            SHAPE_WINDOW,
            SHAPE_FRAME,
        ];
        let risers = [
            SHAPE_STAIRS,
            SHAPE_FOUNDATION_STEPS,
            SHAPE_RAMP,
            SHAPE_STAIRS_L,
            SHAPE_STAIRS_U,
            SHAPE_STAIRS_SPIRAL,
            SHAPE_STAIRS_TRI_SPIRAL,
        ];
        let dense = rng.next_bounded(3);
        for cx in C0..=C0 + N {
            for cz in C0..=C0 + N {
                let inner = cx < C0 + N && cz < C0 + N;
                for level in 0..MAX_BUILD_SOCKETS as u8 {
                    // Sparser up high, so the vertical walks see gaps.
                    if rng.next_bounded(16) < level as u32 / 2 + dense {
                        continue;
                    }
                    if inner {
                        match rng.next_bounded(8) {
                            0 | 1 => {
                                let s = [SHAPE_FOUNDATION, SHAPE_FLOOR, SHAPE_ROOF]
                                    [rng.next_bounded(3) as usize];
                                put(&mut cols, cx, cz, level, LOC_PLANE, s);
                            }
                            2 => put(&mut cols, cx, cz, level, LOC_PLANE, SHAPE_FLOOR_FRAME),
                            3 => {
                                let pair = if rng.next_bounded(2) == 0 {
                                    [LOC_TRI_XLO_ZLO, LOC_TRI_XHI_ZHI]
                                } else {
                                    [LOC_TRI_XHI_ZLO, LOC_TRI_XLO_ZHI]
                                };
                                for loc in pair {
                                    if rng.next_bounded(10) < 7 {
                                        let s = [SHAPE_TRI_FLOOR, SHAPE_TRI_FLOOR_FRAME]
                                            [rng.next_bounded(2) as usize];
                                        put(&mut cols, cx, cz, level, loc, s);
                                    }
                                }
                            }
                            _ => {}
                        }
                        if rng.next_bounded(7) == 0 {
                            let loc = STAIR_LOCS[rng.next_bounded(4) as usize];
                            let s = risers[rng.next_bounded(risers.len() as u32) as usize];
                            put(&mut cols, cx, cz, level, loc, s);
                        }
                        if rng.next_bounded(10) == 0 {
                            let loc = [LOC_DIAG_A, LOC_DIAG_B][rng.next_bounded(2) as usize];
                            let s = edge_shapes[rng.next_bounded(3) as usize];
                            put(&mut cols, cx, cz, level, loc, s);
                        }
                    }
                    for loc in [LOC_EDGE_XLO, LOC_EDGE_ZLO] {
                        let fits = if loc == LOC_EDGE_XLO {
                            cz < C0 + N
                        } else {
                            cx < C0 + N
                        };
                        if fits && rng.next_bounded(4) == 0 {
                            let s = edge_shapes[rng.next_bounded(6) as usize];
                            put(&mut cols, cx, cz, level, loc, s);
                            if matches!(s, SHAPE_DOORWAY | SHAPE_FRAME | SHAPE_WINDOW)
                                && rng.next_bounded(2) == 0
                            {
                                let glass = s == SHAPE_WINDOW && rng.next_bounded(2) == 0;
                                shut.push((cx, cz, level, loc, glass));
                            }
                        }
                    }
                }
            }
        }
        for &(cx, cz, level, loc, glass) in &shut {
            let arch = if glass {
                crate::deploy::ARCH_WINDOW_GLASS
            } else {
                crate::deploy::ARCH_WINDOW_BARS
            };
            cols.set_insert(cx, cz, level, loc, arch, true);
        }
        cols.fill_bands(SEED, hv());
        let mut hulls = Box::new(Hulls::new());
        for &(cx, cz, level, loc, shape) in &placed {
            let p = plate(cx, cz);
            let floor = column_floor_y(SEED, hv(), cx, cz, p) + level_y(level);
            let foot = if matches!(loc, LOC_EDGE_XLO | LOC_EDGE_ZLO) {
                floor - collide::edge_foot_drop(SEED, hv(), &cols, cx, cz, level, loc, p)
            } else {
                floor
            };
            let mut h = grid_hull(
                key(cx, cz, level, loc),
                cx,
                cz,
                level,
                loc,
                shape,
                floor,
                foot,
            );
            if let Some(&(.., glass)) = shut
                .iter()
                .find(|s| (s.0, s.1, s.2, s.3) == (cx, cz, level, loc))
            {
                h.shut = true;
                h.panes = glass;
            }
            assert!(hulls.add(h));
        }
        Base { cols, hulls }
    }

    /// The rows the column index's walks could see for a mover (x, z) →
    /// (nx, nz): the edges and diagonals of the origin's and the
    /// destination's cells, and every body.
    fn cells_view(hulls: &Hulls, x: f32, z: f32, nx: f32, nz: f32) -> Hulls {
        let cell = |v: f32| build_cell_of(v) as u16;
        let ends = [(cell(x), cell(z)), (cell(nx), cell(nz))];
        let mut out = Hulls::new();
        for h in hulls.rows() {
            let (cx, cz) = (h.hit.cx, h.hit.cz);
            let keep = match h.kind {
                HULL_EDGE => ends.iter().any(|&(bx, bz)| {
                    if h.loc == LOC_EDGE_XLO {
                        cz == bz && (cx == bx || cx == bx + 1)
                    } else {
                        cx == bx && (cz == bz || cz == bz + 1)
                    }
                }),
                HULL_DIAG => ends.contains(&(cx, cz)),
                _ => true,
            };
            if keep {
                out.add(*h);
            }
        }
        out
    }

    /// Within a hair of a cell boundary offset by a radius the walks use —
    /// where the column index's `floor((x − r) / 3)` and its own distance
    /// test round differently and the cell walk misses a cell its distance
    /// would take. A seam the hulls do not have; the comparison steps
    /// round it.
    fn near_seam(v: f32) -> bool {
        let f = v - build_cell_of(v) as f32 * BUILD_CELL_M;
        let pad = WALL_THICKNESS_M * 0.5;
        [
            0.0,
            0.03,
            0.15,
            CAPSULE_RADIUS_M,
            pad,
            0.03 + pad,
            0.15 + pad,
            CAPSULE_RADIUS_M + pad,
        ]
        .into_iter()
        .any(|o| {
            [o, BUILD_CELL_M - o, BUILD_CELL_M + o, -o]
                .into_iter()
                .any(|e| crate::fmath::fabs(f - e) < 1e-3)
        })
    }

    fn unit(rng: &mut Pcg32) -> f32 {
        rng.next_bounded(1 << 20) as f32 / (1 << 20) as f32
    }

    #[test]
    fn hulls_answer_what_the_column_index_answers() {
        let (s, hv) = (SEED, hv());
        let mut rng = Pcg32::new(0x5_4AD0, 11);
        let lo = C0 as f32 * BUILD_CELL_M - 1.0;
        let span = N as f32 * BUILD_CELL_M + 2.0;
        let (mut shots, mut shot_ranks, mut ends) = (0, 0, 0);
        for round in 0..4 {
            let b = base(&mut rng);
            let floors: Vec<f32> = b.hulls.rows().iter().map(|h| h.floor_y).collect();
            for i in 0..40_000 {
                let x = lo + unit(&mut rng) * span;
                let z = lo + unit(&mut rng) * span;
                let a = unit(&mut rng) * std::f32::consts::TAU;
                // A body's step and an arrow's tap are under 0.2 m; a
                // longer probe can cross a wall's line past its end, which
                // the column index's own cell walk could not see.
                let d = unit(&mut rng) * 0.19;
                let (sa, ca) = crate::fmath::sin_cos(a);
                let (nx, nz) = (x + d * ca, z + d * sa);
                if [x, z, nx, nz].into_iter().any(near_seam) {
                    continue;
                }
                let feet = floors[rng.next_bounded(floors.len() as u32) as usize]
                    + (unit(&mut rng) - 0.4) * 7.0;
                let ctx = (round, i, x, z, feet);
                assert_eq!(
                    crate::cells_oracle::piece_ground(s, hv, &b.cols, x, z, feet).to_bits(),
                    b.hulls.ground(&STORED, x, z, feet).to_bits(),
                    "ground, {ctx:?}"
                );
                assert_eq!(
                    crate::cells_oracle::body_ground(s, hv, &b.cols, x, z, feet).to_bits(),
                    b.hulls.body_ground(&STORED, x, z, feet).to_bits(),
                    "body_ground, {ctx:?}"
                );
                assert_eq!(
                    crate::cells_oracle::plane_blocked(s, hv, &b.cols, x, z, feet),
                    b.hulls.plane_blocked(&STORED, x, z, feet),
                    "plane_blocked, {ctx:?}"
                );
                assert_eq!(
                    crate::cells_oracle::piece_ceiling(s, hv, &b.cols, x, z, feet)
                        .map(f32::to_bits),
                    b.hulls.ceiling(&STORED, x, z, feet).map(f32::to_bits),
                    "ceiling, {ctx:?}"
                );
                assert_eq!(
                    crate::cells_oracle::floor_under(s, hv, &b.cols, x, z, feet),
                    b.hulls.floor_under(&STORED, x, z, feet),
                    "floor_under, {ctx:?}"
                );
                let ob = crate::cells_oracle::blocked(s, hv, &b.cols, x, z, nx, nz, feet);
                if ob != b.hulls.edges_block(&STORED, x, z, nx, nz, feet) {
                    // Only a wall's end the old walk never looked at — an
                    // edge or diagonal of neither the origin's cell nor the
                    // destination's — may tell them apart.
                    let seen = cells_view(&b.hulls, x, z, nx, nz);
                    assert_eq!(
                        ob,
                        seen.edges_block(&STORED, x, z, nx, nz, feet),
                        "blocked to ({nx}, {nz}), {ctx:?}"
                    );
                    assert!(!ob, "a wall end can only add a block, {ctx:?}");
                    ends += 1;
                }
                let y = feet + unit(&mut rng) * 3.0;
                let r = [0.0, 0.03, 0.15][rng.next_bounded(3) as usize];
                // A tap no longer than the wall's half thickness: within
                // reach of the end it crossed past, as every real tap is
                // of a wall it hits.
                let k = 0.12 / d.max(0.12);
                let (nx, nz) = (x + (nx - x) * k, z + (nz - z) * k);
                let old = crate::cells_oracle::shot_hit(s, hv, &b.cols, x, z, nx, nz, y, r);
                let new = b.hulls.shot_hit(&STORED, x, z, nx, nz, y, r);
                let new = if old.is_some() != new.is_some() {
                    assert!(old.is_none(), "a wall end can only add a hit, {ctx:?}");
                    ends += 1;
                    cells_view(&b.hulls, x, z, nx, nz).shot_hit(&STORED, x, z, nx, nz, y, r)
                } else {
                    new
                };
                assert_eq!(
                    old.is_some(),
                    new.is_some(),
                    "shot to ({nx}, {nz}) y {y} r {r}: {old:?} vs {new:?}, {ctx:?}"
                );
                if let (Some((oh, oi)), Some((nh, ni))) = (old, new) {
                    shots += 1;
                    if (oh, oi) != (nh.hit, ni) {
                        // Two rows stopped one sample: the old walk's own
                        // pick must stop it too, alone.
                        shot_ranks += 1;
                        let mut one = Hulls::new();
                        let k = key(oh.cx, oh.cz, oh.level, oh.loc);
                        one.add(*b.hulls.find(k).expect("old hit has a row"));
                        assert!(
                            one.shot_hit(&STORED, x, z, nx, nz, y, r).is_some(),
                            "old pick {oh:?} is not a hit alone, {ctx:?}"
                        );
                    }
                }
            }
        }
        assert!(shots > 10_000, "the shots must land: {shots}");
        assert!(ends * 200 < 160_000, "wall ends are corners: {ends}");
        assert!(
            shot_ranks * 100 < shots,
            "ties are corners, not the rule: {shot_ranks} of {shots}"
        );
    }
}
