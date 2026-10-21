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
            _ => false,
        }
    }

    /// Edge or diagonal against a mover (x, z) → (nx, nz) with an extra
    /// radius: where along the edge it meets, if it does — `collide.rs`'s
    /// `edge_meet` and `diag_meet`, in the hull's frame.
    fn meet(&self, x: f32, z: f32, nx: f32, nz: f32, r_extra: f32) -> Option<f32> {
        let (d0, d1, along) = if self.kind == HULL_DIAG {
            let (x0, z0) = (self.ox, self.oz);
            if self.loc == LOC_DIAG_B {
                let d = |px: f32, pz: f32| ((px - x0) + (pz - z0) - SIDE_M) * INV_SQRT2;
                let s = ((nx - x0) - (nz - z0) + SIDE_M) * INV_SQRT2;
                (d(x, z), d(nx, nz), s)
            } else {
                let d = |px: f32, pz: f32| ((pz - z0) - (px - x0)) * INV_SQRT2;
                let s = ((nx - x0) + (nz - z0)) * INV_SQRT2;
                (d(x, z), d(nx, nz), s)
            }
        } else {
            let (_, d0) = self.local(x, z);
            let (t, d1) = self.local(nx, nz);
            (d0, d1, t)
        };
        let r = WALL_THICKNESS_M * 0.5 + r_extra;
        let crosses = (d0 < 0.0) != (d1 < 0.0);
        let pushes_in = fabs(d1) < r && fabs(d1) < fabs(d0);
        if !crosses && !pushes_in {
            return None;
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
    rows: Box<[Hull; MAX_PIECES]>,
    len: usize,
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
            rows: crate::boxed_array(Hull::EMPTY),
            len: 0,
            keys: crate::boxed_array(0),
            vals: crate::boxed_array(0),
            space: Space::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn rows(&self) -> &[Hull] {
        &self.rows[..self.len]
    }

    pub fn clear(&mut self) {
        self.rows[..self.len].fill(Hull::EMPTY);
        self.len = 0;
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
        if self.len >= MAX_PIECES {
            return false;
        }
        let i = self.len;
        self.rows[i] = h;
        if !self.file(i) {
            self.rows[i] = Hull::EMPTY;
            return false;
        }
        self.len += 1;
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
        let last = self.len - 1;
        if i != last {
            self.unfile(last);
            self.rows[i] = self.rows[last];
            self.file(i);
            self.put_key(self.rows[i].key, i);
        }
        self.rows[last] = Hull::EMPTY;
        self.len = last;
    }

    /// Open or close the opening of the row with `key`.
    pub fn set_shut(&mut self, key: u64, shut: bool, panes: bool) {
        if let Some(i) = self.index_of(key) {
            self.rows[i].shut = shut;
            self.rows[i].panes = shut && panes;
        }
    }

    /// Visit every row whose filed box meets the world box, possibly more
    /// than once; stops when `f` answers true, and answers whether it did.
    #[inline]
    fn near(&self, x0: f32, z0: f32, x1: f32, z1: f32, mut f: impl FnMut(&Hull) -> bool) -> bool {
        if self.len == 0 {
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
    pub fn ground(&self, x: f32, z: f32, feet_y: f32) -> f32 {
        let lid = feet_y + STEP_UP;
        let mut best = NO_SURFACE;
        self.near(x, z, x, z, |h| {
            let top = match h.kind {
                HULL_PLANE | HULL_HALF | HULL_RISER => {
                    let (lx, lz) = h.local(x, z);
                    if !(0.0..SIDE_M).contains(&lx) || !(0.0..SIDE_M).contains(&lz) {
                        return false;
                    }
                    if h.kind == HULL_RISER {
                        match crate::circulation::surface(h.shape, h.loc, lx, lz) {
                            Some(y) => h.floor_y + y,
                            None => return false,
                        }
                    } else if h.body_solid(lx, lz, 0.0) {
                        h.floor_y
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
    pub fn bodies_near(
        &self,
        x: f32,
        z: f32,
        r: f32,
        mut visit: impl FnMut(usize, f32) -> bool,
    ) -> bool {
        self.near(x - r, z - r, x + r, z + r, |h| {
            if !matches!(h.kind, HULL_PLANE | HULL_HALF | HULL_RISER) {
                return false;
            }
            let (lx, lz) = h.local(x, z);
            if Hull::body_dist2(lx, lz) >= r * r {
                return false;
            }
            if h.kind == HULL_RISER {
                let Some(y) = crate::circulation::surface(h.shape, h.loc, lx, lz) else {
                    return false;
                };
                let slab = if h.shape == SHAPE_FOUNDATION_STEPS {
                    0
                } else {
                    (h.level as usize).max(1)
                };
                return visit(slab, h.floor_y + y);
            }
            let here = match h.kind {
                HULL_PLANE => !h.open || floor_frame_solid(lx, lz, r),
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
            here && visit(h.level as usize, h.floor_y)
        })
    }

    /// The floor a body standing at (`x`, `z`) with its feet at `feet_y`
    /// is on: `collide::floor_under`'s piece walk, the lowest socket at
    /// the first of its five probe points that finds one.
    pub fn floor_under(&self, x: f32, z: f32, feet_y: f32) -> Option<(u16, u16, u8)> {
        const HAND: f32 = 0.25;
        let r = CAPSULE_RADIUS_M;
        for (px, pz) in [(x, z), (x - r, z), (x + r, z), (x, z - r), (x, z + r)] {
            let mut best: Option<(u8, PieceHit)> = None;
            self.near(px, pz, px, pz, |h| {
                if !matches!(h.kind, HULL_PLANE | HULL_HALF | HULL_RISER) {
                    return false;
                }
                let (lx, lz) = h.local(px, pz);
                if !(0.0..SIDE_M).contains(&lx) || !(0.0..SIDE_M).contains(&lz) {
                    return false;
                }
                let y = h.floor_y;
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
    pub fn edges_block(&self, x: f32, z: f32, nx: f32, nz: f32, feet_y: f32) -> bool {
        let reach = CAPSULE_RADIUS_M + WALL_THICKNESS_M * 0.5;
        let (lo, hi) = (feet_y, feet_y + CAPSULE_HEIGHT_M);
        self.near(
            x.min(nx) - reach,
            z.min(nz) - reach,
            x.max(nx) + reach,
            z.max(nz) + reach,
            |h| match h.kind {
                HULL_EDGE => {
                    let bottom = h.floor_y;
                    let foot = h.foot_y;
                    if feet_y >= bottom + h.height() || feet_y + CAPSULE_HEIGHT_M <= foot {
                        return false;
                    }
                    let Some(t) = h.meet(x, z, nx, nz, CAPSULE_RADIUS_M) else {
                        return false;
                    };
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
                    let bottom = h.floor_y;
                    let top = bottom + h.height();
                    if lo >= top || hi <= bottom {
                        return false;
                    }
                    h.meet(x, z, nx, nz, CAPSULE_RADIUS_M)
                        .is_some_and(|t| (0.0..=h.span()).contains(&t))
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
    pub fn shot_edges(
        &self,
        x: f32,
        z: f32,
        nx: f32,
        nz: f32,
        y: f32,
        r: f32,
    ) -> Option<(Hull, bool)> {
        let reach = r + WALL_THICKNESS_M * 0.5;
        let mut best: Option<(Hull, bool)> = None;
        self.near(
            x.min(nx) - reach,
            z.min(nz) - reach,
            x.max(nx) + reach,
            z.max(nz) + reach,
            |h| {
                let got = match h.kind {
                    HULL_EDGE => shot_edge(h, x, z, nx, nz, y, r),
                    HULL_DIAG => {
                        let bottom = h.floor_y;
                        let top = bottom + h.height();
                        if y < bottom || y >= top {
                            None
                        } else {
                            h.meet(x, z, nx, nz, r)
                                .filter(|t| (0.0..=h.span()).contains(t))
                                .map(|_| false)
                        }
                    }
                    _ => None,
                };
                if let Some(insert) = got {
                    if best.is_none_or(|(b, _)| h.rank() < b.rank()) {
                        best = Some((*h, insert));
                    }
                }
                false
            },
        );
        best
    }

    /// Which body or flight a shot sample at (`x`, `z`), altitude `y`,
    /// radius `r` stops on: `collide::cell_planes_stop_shot`. The least
    /// [`Hull::rank`] among several.
    pub fn shot_bodies(&self, x: f32, z: f32, y: f32, r: f32) -> Option<Hull> {
        let mut best: Option<Hull> = None;
        self.near(x - r, z - r, x + r, z + r, |h| {
            if !matches!(h.kind, HULL_PLANE | HULL_HALF | HULL_RISER) {
                return false;
            }
            let (lx, lz) = h.local(x, z);
            if Hull::body_dist2(lx, lz) >= r * r {
                return false;
            }
            let stops = if h.kind == HULL_RISER {
                crate::circulation::surface(h.shape, h.loc, lx, lz).is_some_and(|dy| {
                    let top = h.floor_y + dy;
                    y - r < top
                        && (h.shape == SHAPE_FOUNDATION_STEPS || y + r > top - PLANE_THICKNESS_M)
                })
            } else {
                let here = match h.kind {
                    HULL_PLANE => !h.open || floor_frame_solid(lx, lz, r),
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
                let floor = h.floor_y;
                here && y - r < floor && (h.level == 0 || y + r > floor - PLANE_THICKNESS_M)
            };
            if stops && best.is_none_or(|b| h.rank() < b.rank()) {
                best = Some(*h);
            }
            false
        });
        best
    }
}

impl Hulls {
    /// Whether a body's flank stops a capsule standing at (`x`, `z`) with
    /// its feet at `feet_y`: `collide::plane_blocked`'s piece half.
    pub fn plane_blocked(&self, x: f32, z: f32, feet_y: f32) -> bool {
        let head = feet_y + CAPSULE_HEIGHT_M;
        self.bodies_near(x, z, CAPSULE_RADIUS_M, |level, top| {
            feet_y + STEP_UP < top && (level == 0 || head > top - PLANE_THICKNESS_M)
        })
    }

    /// The lowest slab underside over a capsule's head: `collide::piece_ceiling`.
    pub fn ceiling(&self, x: f32, z: f32, feet_y: f32) -> Option<f32> {
        let head = feet_y + CAPSULE_HEIGHT_M;
        let mut lowest: Option<f32> = None;
        self.bodies_near(x, z, CAPSULE_RADIUS_M, |level, top| {
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
    pub fn body_ground(&self, x: f32, z: f32, feet_y: f32) -> f32 {
        let lid = feet_y + STEP_UP;
        let mut best = NO_SURFACE;
        self.bodies_near(x, z, CAPSULE_RADIUS_M, |_, top| {
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
    pub fn shot_hit(
        &self,
        x: f32,
        z: f32,
        nx: f32,
        nz: f32,
        y: f32,
        r: f32,
    ) -> Option<(Hull, bool)> {
        self.shot_edges(x, z, nx, nz, y, r)
            .or_else(|| self.shot_bodies(nx, nz, y, r).map(|h| (h, false)))
    }
}

/// A straight edge against a shot sample: `collide::cell_edges_stop_shot`'s
/// per-level body. Some(insert) when it stops the shot.
#[allow(clippy::too_many_arguments)]
fn shot_edge(h: &Hull, x: f32, z: f32, nx: f32, nz: f32, y: f32, r: f32) -> Option<bool> {
    let bottom = h.floor_y;
    if y < h.foot_y || y >= bottom + h.height() {
        return None;
    }
    let t = h.meet(x, z, nx, nz, r)?;
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
        assert!(hulls.edges_block(x - 0.6, z, x - 0.1, z, 20.0));
        assert!(!hulls.edges_block(x - 0.6, z + 4.0, x + 0.6, z + 4.0, 20.0));
        assert!(!hulls.edges_block(x - 0.6, z, x - 0.1, z, 23.5), "above it");
    }
}

/// The hulls against the column index they replace: random grid bases of
/// every legacy shape, random bodies and shots, the same answers.
#[cfg(test)]
mod shadow {
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
        (1 << 63) | (cx as u64) << 32 | (cz as u64) << 16 | (level as u64) << 8 | loc as u64
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
                let d = unit(&mut rng) * 0.6;
                let (sa, ca) = crate::fmath::sin_cos(a);
                let (nx, nz) = (x + d * ca, z + d * sa);
                if [x, z, nx, nz].into_iter().any(near_seam) {
                    continue;
                }
                let feet = floors[rng.next_bounded(floors.len() as u32) as usize]
                    + (unit(&mut rng) - 0.4) * 7.0;
                let ctx = (round, i, x, z, feet);
                assert_eq!(
                    collide::piece_ground(s, hv, &b.cols, x, z, feet).to_bits(),
                    b.hulls.ground(x, z, feet).to_bits(),
                    "ground, {ctx:?}"
                );
                assert_eq!(
                    collide::body_ground(s, hv, &b.cols, x, z, feet).to_bits(),
                    b.hulls.body_ground(x, z, feet).to_bits(),
                    "body_ground, {ctx:?}"
                );
                assert_eq!(
                    collide::plane_blocked(s, hv, &b.cols, x, z, feet),
                    b.hulls.plane_blocked(x, z, feet),
                    "plane_blocked, {ctx:?}"
                );
                assert_eq!(
                    collide::piece_ceiling(s, hv, &b.cols, x, z, feet).map(f32::to_bits),
                    b.hulls.ceiling(x, z, feet).map(f32::to_bits),
                    "ceiling, {ctx:?}"
                );
                assert_eq!(
                    collide::floor_under(s, hv, &b.cols, x, z, feet),
                    b.hulls.floor_under(x, z, feet),
                    "floor_under, {ctx:?}"
                );
                let ob = collide::blocked(s, hv, &b.cols, x, z, nx, nz, feet);
                if ob != b.hulls.edges_block(x, z, nx, nz, feet) {
                    // Only a wall's end the old walk never looked at — an
                    // edge or diagonal of neither the origin's cell nor the
                    // destination's — may tell them apart.
                    let seen = cells_view(&b.hulls, x, z, nx, nz);
                    assert_eq!(
                        ob,
                        seen.edges_block(x, z, nx, nz, feet),
                        "blocked to ({nx}, {nz}), {ctx:?}"
                    );
                    assert!(!ob, "a wall end can only add a block, {ctx:?}");
                    ends += 1;
                }
                let y = feet + unit(&mut rng) * 3.0;
                let r = [0.0, 0.03, 0.15][rng.next_bounded(3) as usize];
                let old = collide::shot_hit(s, hv, &b.cols, x, z, nx, nz, y, r);
                let new = b.hulls.shot_hit(x, z, nx, nz, y, r);
                let new = if old.is_some() != new.is_some() {
                    assert!(old.is_none(), "a wall end can only add a hit, {ctx:?}");
                    ends += 1;
                    cells_view(&b.hulls, x, z, nx, nz).shot_hit(x, z, nx, nz, y, r)
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
                            one.shot_hit(x, z, nx, nz, y, r).is_some(),
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
