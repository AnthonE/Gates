//! Piece collision v0 (TERRAIN.md §3 "Buildings: AABB/oriented boxes per
//! block"): placed building pieces become movement collision, shared
//! verbatim by the server tick and the wasm predictor — the client's
//! mirror of the piece store feeds the exact code the server runs, so
//! prediction through a doorway holds bit for bit (skew is bounded by the
//! one in-flight placement event, the same bound the slot store accepts).
//!
//! Geometry: a piece's vertical base is `build::column_floor_y(cell) +
//! level·LEVEL_H_M` — cell-center terrain snapped to the build lattice
//! (`BUILD_BASE_Q_M`) plus the lift, one implementation shared with the
//! renderer. Planes (foundation/floor/roof) are walkable
//! surfaces at their base; stairs are a ramp rising toward +Z through the
//! storey; walls block their edge for the storey they span; doorways
//! block only their posts (the 1.2 m opening passes; the lintel never
//! matters at capsule height until a jump exists). A **closed door**
//! deployable in a doorway blocks the whole edge like a wall; open doors
//! and empty doorways pass. Door state is a shut bit per (column, level,
//! edge), maintained by deploy.rs in lockstep with the door records —
//! derived state like the rest of the index.
//!
//! **Two movers since catalogue v1, one algorithm.** [`blocked`] walks a
//! capsule; [`shot_blocked`] walks a point at its own altitude and radius.
//! The window is why the split exists — solid wall to a body, aperture to
//! an arrow ([`window_solid_at`]) — and the frame (rim only,
//! [`frame_solid_at`]) and the doorway's lintel ([`DOOR_HEAD_M`], shots
//! only) ride the same distinction.
//!
//! The store side is `ColIndex`: an open-addressed, linear-probed map
//! from build column (cx, cz) to per-level occupancy bitmasks, sized so
//! the movement hot path costs O(1) lookups instead of an O(MAX_PIECES)
//! scan per step. It is derived state — rebuilt from the piece store,
//! never hashed, exactly like the event ring — and fixed-capacity: at
//! most one column per piece, so `MAX_PIECES` bounds it below half load.
//!
//! Capsule and slab dimensions are proposed defaults registered in
//! DECISIONS.md §open ("piece collision v0" row).

use crate::build::{
    BUILD_CELL_M, LEVEL_H_M, LOC_DIAG_A, LOC_DIAG_B, LOC_EDGE_XLO, LOC_EDGE_ZLO, LOC_TRI_XHI_ZHI,
    LOC_TRI_XHI_ZLO, LOC_TRI_XLO_ZHI, LOC_TRI_XLO_ZLO, SHAPE_DOORWAY, SHAPE_FLOOR,
    SHAPE_FOUNDATION, SHAPE_FRAME, SHAPE_ROOF, SHAPE_STAIRS, SHAPE_TRI_FLOOR, SHAPE_TRI_FOUNDATION,
    SHAPE_TRI_ROOF, SHAPE_WALL, SHAPE_WINDOW,
};
use crate::fmath::fabs;
use crate::limits::{COL_INDEX_SLOTS, MAX_BUILD_COORD, MAX_BUILD_LEVELS};
use crate::movement::STEP_UP;

/// Foundation top above the cell-center terrain sample. Was render-only
/// (scene.js LIFT); collision makes it sim truth (DECISIONS.md §open,
/// piece collision v0).
pub const PIECE_LIFT_M: f32 = 0.3;
/// Player capsule radius / height (DECISIONS.md §open, piece collision
/// v0; the render capsule is 0.4 m, the eye 1.6 m).
pub const CAPSULE_RADIUS_M: f32 = 0.4;
pub const CAPSULE_HEIGHT_M: f32 = 1.7;
/// Edge-piece slab thickness (scene.js WALL_T, now sim truth).
pub const WALL_THICKNESS_M: f32 = 0.24;
/// Doorway post width from each end of the edge; the opening between is
/// `BUILD_CELL_M − 2·posts` = 1.2 m (scene.js posts, now sim truth).
pub const DOOR_POST_W_M: f32 = 0.9;
/// The window's aperture band, metres above the storey base: sill below
/// it, header above it, jambs [`DOOR_POST_W_M`] in from each end — the
/// doorway's posts reused rather than a second width, so the two openings
/// stay one family. 1.2 m tall × 1.2 m wide: an arrow threads it, a
/// 1.7 m capsule never does, which is the window's whole collision
/// contract — **it blocks a body and not a shot** (`NOW.md` §0ac's stated
/// answer; proposed defaults, DECISIONS.md §open "window v0").
pub const WINDOW_SILL_M: f32 = 1.0;
pub const WINDOW_HEAD_M: f32 = 2.2;
/// The wall frame's rim, metres in from each end of its edge (and down
/// from its top): the only solid the frame has until an insert fills it.
/// Thin enough that the opening reads as the whole edge, thick enough to
/// be drawn — and it is drawn, which is why it must block: the doorway's
/// law is that the frame may not lie about where a player can walk
/// (`RENDER.md` §8), and a painted jamb a body ghosts through is that lie.
/// Proposed default, DECISIONS.md §open ("wall frame v0").
pub const FRAME_RIM_M: f32 = 0.15;
/// The doorway opening's height, metres above the storey base — the
/// lintel's underside. The client has always drawn the lintel at
/// 2.1..3.0 (`render/structures.rs` derives it from `LINTEL_H_M`); the
/// sim never modelled it because a 1.7 m capsule cannot reach it without
/// a jump, and that is still true for bodies. The **shot** walk is what
/// finally reads it: an arrow is not a capsule, and an arrow through the
/// drawn lintel was the frame lying to the other sense.
pub const DOOR_HEAD_M: f32 = 2.1;

/// "No built surface here" sentinel — far below any terrain, so
/// `max(terrain, piece_ground)` needs no branch.
pub const NO_SURFACE: f32 = -1.0e9;

/// "No solid deployable at any level" — every nibble of [`ColMasks::solid`]
/// at the sentinel. All-ones rather than zero because zero is a real
/// archetype (`deploy::ARCH_BAG`), which is also why `Default` below is a
/// hand impl and not a derive.
pub const SOLID_NONE: u32 = u32::MAX;

/// Per-column occupancy, one bit per level (MAX_BUILD_LEVELS = 8 fits u8
/// exactly). Edge masks live in their canonical column (build.rs: low-x/
/// low-z), so a cell's +x boundary is its +x neighbor's `*_xlo`.
///
/// The suffixes were `_w`/`_n` until 2026-08-15 and named bearings that had
/// since moved (`build.rs`'s `LOC_EDGE_XLO` doc). Axes, now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ColMasks {
    pub planes: u8,
    pub stairs: u8,
    pub walls_xlo: u8,
    pub walls_zlo: u8,
    pub doors_xlo: u8,
    pub doors_zlo: u8,
    /// Window edges (catalogue v1): a body-block the movement walk treats
    /// as a wall and the shot walk treats as a wall with an aperture
    /// (`WINDOW_SILL_M..WINDOW_HEAD_M`, jambs at the doorway's posts).
    pub wins_xlo: u8,
    pub wins_zlo: u8,
    /// Wall-frame edges: no collision at all until an insert exists — the
    /// bits are here so occupancy (`build::occupied_at`, the collapse
    /// cascade, the claim probe) can see the piece, not so anything
    /// blocks on it.
    pub frames_xlo: u8,
    pub frames_zlo: u8,
    /// Triangle half-planes (triangles v0): standable ground over their
    /// own half of the cell, nothing over the other (`piece_ground`'s
    /// half tests). Named by the corner the right angle sits in —
    /// `build::LOC_TRI_*`'s map.
    pub tri_xlo_zlo: u8,
    pub tri_xhi_zlo: u8,
    pub tri_xlo_zhi: u8,
    pub tri_xhi_zhi: u8,
    /// Diagonal walls: full-span blocks along their own line, to bodies
    /// and shots alike, wholly inside the cell.
    pub diag_a: u8,
    pub diag_b: u8,
    /// Closed-door bits: set ⇒ the doorway at this level/edge holds a
    /// closed door deployable and blocks its full span (deploy.rs keeps
    /// these in lockstep with the door records).
    pub shut_xlo: u8,
    pub shut_zlo: u8,
    /// The solid deployable standing on each level's plane, one nibble
    /// per level: the archetype code, or `0xF` for none (deploy collision
    /// v0 — deploy.rs keeps these in lockstep with the deploy records,
    /// exactly as it keeps the shut bits). The *volume* the code names is
    /// `deploy::DEPLOY_VOL`'s row; only archetypes that table gives a
    /// height ever land here.
    pub solid: u32,
}

impl Default for ColMasks {
    fn default() -> Self {
        Self::EMPTY
    }
}

impl ColMasks {
    pub const EMPTY: Self = Self {
        planes: 0,
        stairs: 0,
        walls_xlo: 0,
        walls_zlo: 0,
        doors_xlo: 0,
        doors_zlo: 0,
        wins_xlo: 0,
        wins_zlo: 0,
        frames_xlo: 0,
        frames_zlo: 0,
        tri_xlo_zlo: 0,
        tri_xhi_zlo: 0,
        tri_xlo_zhi: 0,
        tri_xhi_zhi: 0,
        diag_a: 0,
        diag_b: 0,
        shut_xlo: 0,
        shut_zlo: 0,
        solid: SOLID_NONE,
    };

    fn is_empty(&self) -> bool {
        (self.planes
            | self.stairs
            | self.walls_xlo
            | self.walls_zlo
            | self.doors_xlo
            | self.doors_zlo
            | self.wins_xlo
            | self.wins_zlo
            | self.frames_xlo
            | self.frames_zlo
            | self.tri_xlo_zlo
            | self.tri_xhi_zlo
            | self.tri_xlo_zhi
            | self.tri_xhi_zhi
            | self.diag_a
            | self.diag_b
            | self.shut_xlo
            | self.shut_zlo)
            == 0
            && self.solid == SOLID_NONE
    }

    /// The solid archetype standing at `level`, or `None`.
    #[inline]
    pub fn solid_at(&self, level: usize) -> Option<u8> {
        let nib = (self.solid >> (level * 4)) & 0xF;
        (nib != 0xF).then_some(nib as u8)
    }

    /// The mask a (shape, loc) pair lives in, or None for shapes with no
    /// collision footprint.
    fn field(&mut self, shape: u8, loc: u8) -> Option<&mut u8> {
        match shape {
            SHAPE_FOUNDATION | SHAPE_FLOOR | SHAPE_ROOF => Some(&mut self.planes),
            SHAPE_STAIRS => Some(&mut self.stairs),
            SHAPE_WALL if loc == LOC_EDGE_XLO => Some(&mut self.walls_xlo),
            SHAPE_WALL if loc == LOC_EDGE_ZLO => Some(&mut self.walls_zlo),
            SHAPE_WALL if loc == LOC_DIAG_A => Some(&mut self.diag_a),
            SHAPE_WALL if loc == LOC_DIAG_B => Some(&mut self.diag_b),
            SHAPE_DOORWAY if loc == LOC_EDGE_XLO => Some(&mut self.doors_xlo),
            SHAPE_DOORWAY if loc == LOC_EDGE_ZLO => Some(&mut self.doors_zlo),
            SHAPE_WINDOW if loc == LOC_EDGE_XLO => Some(&mut self.wins_xlo),
            SHAPE_WINDOW if loc == LOC_EDGE_ZLO => Some(&mut self.wins_zlo),
            SHAPE_FRAME if loc == LOC_EDGE_XLO => Some(&mut self.frames_xlo),
            SHAPE_FRAME if loc == LOC_EDGE_ZLO => Some(&mut self.frames_zlo),
            SHAPE_TRI_FOUNDATION | SHAPE_TRI_FLOOR | SHAPE_TRI_ROOF => match loc {
                LOC_TRI_XLO_ZLO => Some(&mut self.tri_xlo_zlo),
                LOC_TRI_XHI_ZLO => Some(&mut self.tri_xhi_zlo),
                LOC_TRI_XLO_ZHI => Some(&mut self.tri_xlo_zhi),
                LOC_TRI_XHI_ZHI => Some(&mut self.tri_xhi_zhi),
                _ => None,
            },
            _ => None,
        }
    }
}

/// Open-addressed column map, linear probing, backward-shift deletion —
/// no tombstones, so a shard that builds and decays for months never
/// degrades. Fixed capacity (limits.rs `COL_INDEX_SLOTS` = 2 × the piece
/// cap), keys packed `1<<31 | cx<<10 | cz` so 0 means empty.
///
/// The two arrays are boxed (`crate::boxed_array`) — CLAUDE.md's
/// stack-frame trap, met a fourth time: they lived inline and
/// `ColIndex::new()` materialised them in a frame, which fit while a mask
/// row was 12 bytes and blew a test thread's 2 MiB stack the day
/// catalogue v1 made it 16. Fill on the heap and the constructor's frame
/// is two pointers, whatever the masks grow to next.
pub struct ColIndex {
    keys: Box<[u32; COL_INDEX_SLOTS]>,
    masks: Box<[ColMasks; COL_INDEX_SLOTS]>,
    len: u32,
}

const OCCUPIED: u32 = 1 << 31;

impl ColIndex {
    pub fn new() -> Self {
        Self {
            keys: crate::boxed_array(0),
            masks: crate::boxed_array(ColMasks::EMPTY),
            len: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.len as usize
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn clear(&mut self) {
        // `fill`, not array assignment: `*self.keys = [0; N]` builds the
        // replacement array in this frame first, which is the exact trap
        // the boxes exist to close.
        self.keys.fill(0);
        self.masks.fill(ColMasks::EMPTY);
        self.len = 0;
    }

    #[inline]
    fn key(cx: u16, cz: u16) -> u32 {
        OCCUPIED | ((cx as u32) << 10) | cz as u32
    }

    /// Fibonacci-hash home slot — pure integer, wasm-identical.
    #[inline]
    fn home(key: u32) -> usize {
        (key.wrapping_mul(0x9E37_79B1) >> 18) as usize & (COL_INDEX_SLOTS - 1)
    }

    /// The column's masks; EMPTY when nothing is built there. The probe
    /// terminates at the first empty slot (the table never fills: one
    /// column per piece keeps it under half load).
    pub fn get(&self, cx: u16, cz: u16) -> ColMasks {
        let key = Self::key(cx, cz);
        let mut i = Self::home(key);
        loop {
            let k = self.keys[i];
            if k == 0 {
                return ColMasks::EMPTY;
            }
            if k == key {
                return self.masks[i];
            }
            i = (i + 1) & (COL_INDEX_SLOTS - 1);
        }
    }

    /// Set the piece's occupancy bit. A full table refuses (unreachable
    /// while callers enforce MAX_PIECES; the movement queries just see
    /// one piece less — the same bounded staleness a dropped event has).
    pub fn add(&mut self, cx: u16, cz: u16, level: u8, loc: u8, shape: u8) {
        if self.len as usize >= COL_INDEX_SLOTS - 1 {
            return;
        }
        let key = Self::key(cx, cz);
        let mut i = Self::home(key);
        loop {
            let k = self.keys[i];
            if k == key {
                break;
            }
            if k == 0 {
                self.keys[i] = key;
                self.len += 1;
                break;
            }
            i = (i + 1) & (COL_INDEX_SLOTS - 1);
        }
        if let Some(m) = self.masks[i].field(shape, loc) {
            *m |= 1 << level;
        } else if self.masks[i].is_empty() {
            // A no-footprint shape opened this slot: take it back.
            self.remove_slot(i);
        }
    }

    /// Clear the piece's occupancy bit; an emptied column leaves the
    /// table entirely (backward-shift, so probes stay short forever).
    pub fn del(&mut self, cx: u16, cz: u16, level: u8, loc: u8, shape: u8) {
        let key = Self::key(cx, cz);
        let mut i = Self::home(key);
        loop {
            let k = self.keys[i];
            if k == 0 {
                return;
            }
            if k == key {
                break;
            }
            i = (i + 1) & (COL_INDEX_SLOTS - 1);
        }
        if let Some(m) = self.masks[i].field(shape, loc) {
            *m &= !(1 << level);
        }
        if self.masks[i].is_empty() {
            self.remove_slot(i);
        }
    }

    /// Set or clear a closed-door bit (deploy.rs: a door placing, its
    /// doorway decaying it away, or a use toggle). A non-edge loc is a
    /// no-op — the caller validated the address holds a door. Clearing
    /// an emptied column drops its slot like `del`.
    pub fn set_door(&mut self, cx: u16, cz: u16, level: u8, loc: u8, shut: bool) {
        if loc != LOC_EDGE_XLO && loc != LOC_EDGE_ZLO {
            return;
        }
        if !shut {
            // Clear: reuse the del-style walk; absent column means clear.
            let key = Self::key(cx, cz);
            let mut i = Self::home(key);
            loop {
                let k = self.keys[i];
                if k == 0 {
                    return;
                }
                if k == key {
                    break;
                }
                i = (i + 1) & (COL_INDEX_SLOTS - 1);
            }
            let m = &mut self.masks[i];
            if loc == LOC_EDGE_XLO {
                m.shut_xlo &= !(1 << level);
            } else {
                m.shut_zlo &= !(1 << level);
            }
            if self.masks[i].is_empty() {
                self.remove_slot(i);
            }
            return;
        }
        if self.len as usize >= COL_INDEX_SLOTS - 1 {
            return; // full-table posture matches add(): bounded staleness
        }
        let key = Self::key(cx, cz);
        let mut i = Self::home(key);
        loop {
            let k = self.keys[i];
            if k == key {
                break;
            }
            if k == 0 {
                self.keys[i] = key;
                self.len += 1;
                break;
            }
            i = (i + 1) & (COL_INDEX_SLOTS - 1);
        }
        let m = &mut self.masks[i];
        if loc == LOC_EDGE_XLO {
            m.shut_xlo |= 1 << level;
        } else {
            m.shut_zlo |= 1 << level;
        }
    }

    /// Set or clear the solid-deployable nibble at (column, level) —
    /// deploy.rs's lockstep write, `set_door`'s shape (deploy collision
    /// v0). `arch` must already have a volume (`deploy::solid_vol`); the
    /// writer checks, because this index stores codes and does not know
    /// the table.
    pub fn set_solid(&mut self, cx: u16, cz: u16, level: u8, arch: Option<u8>) {
        let shift = (level as usize & 7) * 4;
        let Some(a) = arch else {
            // Clear: absent column already means clear.
            let key = Self::key(cx, cz);
            let mut i = Self::home(key);
            loop {
                let k = self.keys[i];
                if k == 0 {
                    return;
                }
                if k == key {
                    break;
                }
                i = (i + 1) & (COL_INDEX_SLOTS - 1);
            }
            self.masks[i].solid |= 0xF << shift;
            if self.masks[i].is_empty() {
                self.remove_slot(i);
            }
            return;
        };
        if self.len as usize >= COL_INDEX_SLOTS - 1 {
            return; // full-table posture matches add(): bounded staleness
        }
        let key = Self::key(cx, cz);
        let mut i = Self::home(key);
        loop {
            let k = self.keys[i];
            if k == key {
                break;
            }
            if k == 0 {
                self.keys[i] = key;
                self.len += 1;
                break;
            }
            i = (i + 1) & (COL_INDEX_SLOTS - 1);
        }
        self.masks[i].solid = (self.masks[i].solid & !(0xF << shift)) | ((a as u32 & 0xF) << shift);
    }

    /// Knuth 6.4 R: refill the hole from the probe chain behind it.
    fn remove_slot(&mut self, mut i: usize) {
        self.keys[i] = 0;
        self.masks[i] = ColMasks::EMPTY;
        self.len -= 1;
        let mut j = i;
        loop {
            j = (j + 1) & (COL_INDEX_SLOTS - 1);
            let k = self.keys[j];
            if k == 0 {
                return;
            }
            let h = Self::home(k);
            // Slot j may move into the hole iff its home does not lie
            // strictly between the hole and j in probe order.
            let jh = j.wrapping_sub(h) & (COL_INDEX_SLOTS - 1);
            let ji = j.wrapping_sub(i) & (COL_INDEX_SLOTS - 1);
            if jh >= ji {
                self.keys[i] = k;
                self.masks[i] = self.masks[j];
                self.keys[j] = 0;
                self.masks[j] = ColMasks::EMPTY;
                i = j;
            }
        }
    }
}

impl Default for ColIndex {
    fn default() -> Self {
        Self::new()
    }
}

/// A column's level-0 base — `build::column_floor_y`, the one
/// implementation of the height rule (quantized to `BUILD_BASE_Q_M` so
/// neighbouring columns in one terrain band are bit-equal flush; its doc
/// carries the derivation). No piece height ever rides the wire.
#[inline]
pub(crate) fn col_base_y(seed: u64, cx: u16, cz: u16) -> f32 {
    crate::build::column_floor_y(seed, cx, cz)
}

/// The highest built surface under (x, z) the capsule at `feet_y` may
/// stand on — a plane's top, or the stair ramp's height at this z —
/// `NO_SURFACE` when none. "May stand on" is the step rule: a surface
/// more than STEP_UP above the feet is a ceiling, not a floor (walking
/// under a level-2 floor must not teleport anyone up).
pub fn piece_ground(seed: u64, cols: &ColIndex, x: f32, z: f32, feet_y: f32) -> f32 {
    let bx = crate::build::build_cell_of(x);
    let bz = crate::build::build_cell_of(z);
    if bx < 0 || bz < 0 || bx >= MAX_BUILD_COORD as i32 || bz >= MAX_BUILD_COORD as i32 {
        return NO_SURFACE;
    }
    let m = cols.get(bx as u16, bz as u16);
    let tris = m.tri_xlo_zlo | m.tri_xhi_zlo | m.tri_xlo_zhi | m.tri_xhi_zhi;
    if m.planes == 0 && m.stairs == 0 && tris == 0 && m.solid == SOLID_NONE {
        return NO_SURFACE;
    }
    let base = col_base_y(seed, bx as u16, bz as u16);
    let lid = feet_y + STEP_UP;
    let mut best = NO_SURFACE;
    // A triangle plane is ground over its own half of the cell and air
    // over the other (triangles v0). The half tests are the address
    // definitions themselves (`build::LOC_TRI_*`): boundary-inclusive on
    // both sides of a diagonal, so a body on the seam of a NW+SE pair
    // reads the same floor from either.
    let dx = x - bx as f32 * BUILD_CELL_M;
    let dz = z - bz as f32 * BUILD_CELL_M;
    let in_half = |loc: u8| match loc {
        LOC_TRI_XLO_ZLO => dx + dz <= BUILD_CELL_M,
        LOC_TRI_XHI_ZHI => dx + dz >= BUILD_CELL_M,
        LOC_TRI_XHI_ZLO => dz <= dx,
        _ => dz >= dx,
    };
    // Solid-deploy tops are standable ground (deploy collision v0): the
    // reference's box-stair. The footprint is the volume's own — not
    // inflated by the capsule — so a body stands on a furnace only with
    // its centre over the furnace, the same rule a cell boundary already
    // applies to a floor slab. The lid keeps a tall top from teleporting
    // anyone up: a box (0.65) needs the jump, which clears it.
    let (cxm, czm) = (
        bx as f32 * BUILD_CELL_M + BUILD_CELL_M * 0.5,
        bz as f32 * BUILD_CELL_M + BUILD_CELL_M * 0.5,
    );
    for level in 0..MAX_BUILD_LEVELS {
        let bit = 1u8 << level;
        let floor = base + level as f32 * LEVEL_H_M;
        if m.planes & bit != 0 && floor <= lid && floor > best {
            best = floor;
        }
        if tris & bit != 0 && floor <= lid && floor > best {
            let on = (m.tri_xlo_zlo & bit != 0 && in_half(LOC_TRI_XLO_ZLO))
                || (m.tri_xhi_zlo & bit != 0 && in_half(LOC_TRI_XHI_ZLO))
                || (m.tri_xlo_zhi & bit != 0 && in_half(LOC_TRI_XLO_ZHI))
                || (m.tri_xhi_zhi & bit != 0 && in_half(LOC_TRI_XHI_ZHI));
            if on {
                best = floor;
            }
        }
        if m.stairs & bit != 0 {
            // The ramp rises toward +Z through the storey (scene.js).
            let frac = ((z - bz as f32 * BUILD_CELL_M) / BUILD_CELL_M).clamp(0.0, 1.0);
            let ramp = floor + frac * LEVEL_H_M;
            if ramp <= lid && ramp > best {
                best = ramp;
            }
        }
        if let Some(arch) = m.solid_at(level) {
            if let Some((hw, h, hd)) = crate::deploy::solid_vol(arch) {
                let top = floor + h;
                if fabs(x - cxm) <= hw && fabs(z - czm) <= hd && top <= lid && top > best {
                    best = top;
                }
            }
        }
    }
    best
}

/// Whether a solid deployable stops a capsule standing at (`x`, `z`) with
/// its feet at `feet_y` (deploy collision v0). A destination test like
/// `occupy::Occupants::blocks`, and complete over the candidate's own
/// build cell alone: `deploy::DEPLOY_VOL`'s const block proves no volume,
/// inflated by the capsule, reaches past the half-cell. The XZ test is
/// the clamp-to-rectangle circle distance `terrain::boxes_block` uses,
/// for its reason — growing the rectangle by the radius rounds corners
/// the wrong way.
pub fn deploy_blocked(seed: u64, cols: &ColIndex, x: f32, z: f32, feet_y: f32) -> bool {
    let bx = crate::build::build_cell_of(x);
    let bz = crate::build::build_cell_of(z);
    if bx < 0 || bz < 0 || bx >= MAX_BUILD_COORD as i32 || bz >= MAX_BUILD_COORD as i32 {
        return false;
    }
    let m = cols.get(bx as u16, bz as u16);
    if m.solid == SOLID_NONE {
        return false;
    }
    let base = col_base_y(seed, bx as u16, bz as u16);
    let head = feet_y + CAPSULE_HEIGHT_M;
    let (cxm, czm) = (
        bx as f32 * BUILD_CELL_M + BUILD_CELL_M * 0.5,
        bz as f32 * BUILD_CELL_M + BUILD_CELL_M * 0.5,
    );
    for level in 0..MAX_BUILD_LEVELS {
        let Some(arch) = m.solid_at(level) else {
            continue;
        };
        let Some((hw, h, hd)) = crate::deploy::solid_vol(arch) else {
            continue;
        };
        let bottom = base + level as f32 * LEVEL_H_M;
        // A top within STEP_UP of the feet is a step, not a wall — the
        // rule `piece_ground` already applies to every standable surface
        // (its lid), extended here so the horizontal pass admits the move
        // the vertical pass would land: the body mounts the top exactly
        // as it mounts a lifted slab. It read `feet_y >= bottom + h`
        // (top-or-above only) until 2026-08-16, when the build lattice
        // exposed the asymmetry: a ground box's top rides up to q/2
        // higher than it used to, and a 1.20 m top against the jump's
        // 1.22 m apex left `a_jump_lands_on_the_box_top` a 2 cm window —
        // the two passes disagreeing about whether the top was reachable.
        // Walking in from flat ground is still blocked (a box top sits
        // ≥ 0.7 above the feet, past the step), so the jump stays the
        // verb that mounts one.
        if feet_y + STEP_UP >= bottom + h || head <= bottom {
            continue;
        }
        let qx = (x - cxm).clamp(-hw, hw);
        let qz = (z - czm).clamp(-hd, hd);
        let ex = x - cxm - qx;
        let ez = z - czm - qz;
        if ex * ex + ez * ez < CAPSULE_RADIUS_M * CAPSULE_RADIUS_M {
            return true;
        }
    }
    false
}

/// Where the move `a0`→`a1` meets the edge plane at `px`, if it does: the
/// along-edge metre of the meeting, measured from `s0`. Coordinates are
/// pre-swapped by the caller so the math is written once for both edge
/// axes. `r_extra` inflates the slab by the mover's own radius — the
/// capsule's for a body ([`cell_edges_block`]), the arrowhead's for a shot
/// ([`shot_blocked`]), which is the radius parameter `ranged.rs`'s module
/// doc spent a paragraph owing. What is solid at the returned metre is the
/// caller's per-shape question ([`doorway_solid_at`] and family).
fn edge_meet(px: f32, s0: f32, a0: f32, a1: f32, along: f32, r_extra: f32) -> Option<f32> {
    let r = WALL_THICKNESS_M * 0.5 + r_extra;
    let d0 = a0 - px;
    let d1 = a1 - px;
    let crosses = (d0 < 0.0 && d1 > 0.0) || (d0 > 0.0 && d1 < 0.0);
    let pushes_in = fabs(d1) < r && fabs(d1) < fabs(d0);
    if !crosses && !pushes_in {
        return None;
    }
    Some(along - s0)
}

/// Is a doorway SOLID at `t` metres along its edge? The two posts block; the
/// gap between them is the opening.
///
/// **Extracted so the renderer can be gated against it rather than against a
/// copy of it.** `render/ghost.rs` and `render/structures.rs` both draw this
/// doorway, and `crates/client/tests/ghost.rs` asserts the drawn posts land
/// exactly where this returns true. `RENDER.md` §8 states the stake: the
/// opening is what `cell_edges_block` refuses to block, and "draw it elsewhere
/// and the frame lies about where a player can walk". A test that restated this
/// band instead of calling it would go green while the two drifted, which is the
/// byte-golden hole `CLAUDE.md`'s trap list names.
///
/// Pure, allocation-free, comparison-only — wall 1 is untouched.
pub fn doorway_solid_at(t: f32) -> bool {
    (0.0..=DOOR_POST_W_M).contains(&t) || (BUILD_CELL_M - DOOR_POST_W_M..=BUILD_CELL_M).contains(&t)
}

/// Is a window SOLID at `t` metres along its edge and `y` metres above its
/// storey base? Everything but the aperture: jambs outside the doorway's
/// post span, sill below [`WINDOW_SILL_M`], header above [`WINDOW_HEAD_M`].
///
/// `pub` for [`doorway_solid_at`]'s exact reason: the renderer draws this
/// window (`render/structures.rs`) and the drawn solids are gated against
/// this function rather than against a copy of its arithmetic
/// (`crates/client/tests/ghost.rs`). Only the **shot** walk consults it —
/// to a body the whole edge is solid (`cell_edges_block`).
pub fn window_solid_at(t: f32, y: f32) -> bool {
    doorway_solid_at(t) || !(WINDOW_SILL_M..=WINDOW_HEAD_M).contains(&y)
}

/// Is a wall frame SOLID at `t` metres along its edge and `y` metres above
/// its storey base? Only its rim: [`FRAME_RIM_M`] in from each end, and the
/// top beam [`FRAME_RIM_M`] down from the storey ceiling. `pub` for the
/// same render gate the other two carry.
pub fn frame_solid_at(t: f32, y: f32) -> bool {
    (0.0..=FRAME_RIM_M).contains(&t)
        || (BUILD_CELL_M - FRAME_RIM_M..=BUILD_CELL_M).contains(&t)
        || y >= LEVEL_H_M - FRAME_RIM_M
}

/// All blocking edges of one cell against the move. The four edges are
/// the cell's canonical low-x/low-z plus the neighbors' (build.rs edge
/// canonicalization); each tests only if its storey overlaps the capsule.
#[allow(clippy::too_many_arguments)]
fn cell_edges_block(
    seed: u64,
    cols: &ColIndex,
    bx: i32,
    bz: i32,
    x: f32,
    z: f32,
    nx: f32,
    nz: f32,
    feet_y: f32,
) -> bool {
    // (column cx, cz, x-plane?) — the four boundaries of cell (bx, bz).
    let edges = [
        (bx, bz, true),
        (bx + 1, bz, true),
        (bx, bz, false),
        (bx, bz + 1, false),
    ];
    for (ecx, ecz, x_plane) in edges {
        if ecx < 0 || ecz < 0 || ecx >= MAX_BUILD_COORD as i32 || ecz >= MAX_BUILD_COORD as i32 {
            continue;
        }
        let m = cols.get(ecx as u16, ecz as u16);
        // Windows ride the wall mask here on purpose: to a moving body a
        // window IS a wall (the aperture is above a capsule's reach and
        // under its head — `WINDOW_SILL_M`'s doc). A frame blocks only its
        // thin rim jambs, and its top beam not at all — the lintel
        // precedent: nothing at capsule height until a jump exists. The
        // shot walk (`shot_blocked`) is where the shapes fully diverge.
        let (walls, doors, frames, shuts) = if x_plane {
            (
                m.walls_xlo | m.wins_xlo,
                m.doors_xlo,
                m.frames_xlo,
                m.shut_xlo,
            )
        } else {
            (
                m.walls_zlo | m.wins_zlo,
                m.doors_zlo,
                m.frames_zlo,
                m.shut_zlo,
            )
        };
        if walls | doors | frames == 0 {
            continue;
        }
        let base = col_base_y(seed, ecx as u16, ecz as u16);
        for level in 0..MAX_BUILD_LEVELS {
            let bit = 1u8 << level;
            let has_wall = walls & bit != 0;
            let has_door = doors & bit != 0;
            let has_frame = frames & bit != 0;
            if !has_wall && !has_door && !has_frame {
                continue;
            }
            let bottom = base + level as f32 * LEVEL_H_M;
            if feet_y >= bottom + LEVEL_H_M || feet_y + CAPSULE_HEIGHT_M <= bottom {
                continue; // that storey is above the head or below the feet
            }
            let meet = if x_plane {
                // x-plane at ecx·3, spanning z from ecz·3.
                let px = ecx as f32 * BUILD_CELL_M;
                let s0 = ecz as f32 * BUILD_CELL_M;
                edge_meet(px, s0, x, nx, nz, CAPSULE_RADIUS_M)
            } else {
                // z-plane at ecz·3, spanning x from ecx·3.
                let pz = ecz as f32 * BUILD_CELL_M;
                let s0 = ecx as f32 * BUILD_CELL_M;
                edge_meet(pz, s0, z, nz, nx, CAPSULE_RADIUS_M)
            };
            let Some(t) = meet else { continue };
            let hit = if has_wall {
                (0.0..=BUILD_CELL_M).contains(&t)
            } else if has_door {
                // A closed door seals the doorway: full span, like a wall.
                if shuts & bit != 0 {
                    (0.0..=BUILD_CELL_M).contains(&t)
                } else {
                    doorway_solid_at(t)
                }
            } else {
                // The frame's jambs, at any capsule height in the storey.
                (0.0..=FRAME_RIM_M).contains(&t)
                    || (BUILD_CELL_M - FRAME_RIM_M..=BUILD_CELL_M).contains(&t)
            };
            if hit {
                return true;
            }
        }
    }
    false
}

/// 1/√2 and the diagonal's length — the two constants the rotated-frame
/// arithmetic below needs, written as literals because wall 1 has no
/// sqrt-at-runtime to spend on a compile-time number.
const INV_SQRT2: f32 = std::f32::consts::FRAC_1_SQRT_2;
const DIAG_LEN_M: f32 = BUILD_CELL_M * std::f32::consts::SQRT_2;

/// Where the move `a`→`b` meets one diagonal of cell (bx, bz), if it
/// does: the along-diagonal metre of the meeting in `0..DIAG_LEN_M`.
/// [`edge_meet`]'s exact algebra in a frame turned 45° — the signed
/// distance to the line and the coordinate along it are one rotation,
/// all multiplies (triangles v0).
#[allow(clippy::too_many_arguments)]
fn diag_meet(
    bx: i32,
    bz: i32,
    diag_b: bool,
    x: f32,
    z: f32,
    nx: f32,
    nz: f32,
    r_extra: f32,
) -> Option<f32> {
    let x0 = bx as f32 * BUILD_CELL_M;
    let z0 = bz as f32 * BUILD_CELL_M;
    let (d0, d1, along) = if diag_b {
        // Line dx + dz = cell: distance grows toward SE.
        let d = |px: f32, pz: f32| ((px - x0) + (pz - z0) - BUILD_CELL_M) * INV_SQRT2;
        let s = ((nx - x0) - (nz - z0) + BUILD_CELL_M) * INV_SQRT2;
        (d(x, z), d(nx, nz), s)
    } else {
        // Line dz = dx: distance grows toward SW.
        let d = |px: f32, pz: f32| ((pz - z0) - (px - x0)) * INV_SQRT2;
        let s = ((nx - x0) + (nz - z0)) * INV_SQRT2;
        (d(x, z), d(nx, nz), s)
    };
    let r = WALL_THICKNESS_M * 0.5 + r_extra;
    let crosses = (d0 < 0.0 && d1 > 0.0) || (d0 > 0.0 && d1 < 0.0);
    let pushes_in = fabs(d1) < r && fabs(d1) < fabs(d0);
    if !crosses && !pushes_in {
        return None;
    }
    Some(along)
}

/// The diagonal walls of one cell against a mover — the interior twin of
/// the boundary walks, called for both endpoint cells by [`blocked`] and
/// [`shot_blocked`]. The vertical test matches each caller's own: a
/// capsule OVERLAPS a storey (`point` false, `lo_y..hi_y` the body), a
/// shot is CONTAINED in one (`point` true, both bounds the altitude).
#[allow(clippy::too_many_arguments)]
fn cell_diags_block(
    seed: u64,
    cols: &ColIndex,
    bx: i32,
    bz: i32,
    x: f32,
    z: f32,
    nx: f32,
    nz: f32,
    lo_y: f32,
    hi_y: f32,
    point: bool,
    r_extra: f32,
) -> bool {
    if bx < 0 || bz < 0 || bx >= MAX_BUILD_COORD as i32 || bz >= MAX_BUILD_COORD as i32 {
        return false;
    }
    let m = cols.get(bx as u16, bz as u16);
    if m.diag_a | m.diag_b == 0 {
        return false;
    }
    let base = col_base_y(seed, bx as u16, bz as u16);
    for level in 0..MAX_BUILD_LEVELS {
        let bit = 1u8 << level;
        if (m.diag_a | m.diag_b) & bit == 0 {
            continue;
        }
        let bottom = base + level as f32 * LEVEL_H_M;
        let outside = if point {
            lo_y < bottom || lo_y >= bottom + LEVEL_H_M
        } else {
            lo_y >= bottom + LEVEL_H_M || hi_y <= bottom
        };
        if outside {
            continue;
        }
        for (mask, diag_b) in [(m.diag_a, false), (m.diag_b, true)] {
            if mask & bit == 0 {
                continue;
            }
            if let Some(t) = diag_meet(bx, bz, diag_b, x, z, nx, nz, r_extra) {
                if (0.0..=DIAG_LEN_M).contains(&t) {
                    return true;
                }
            }
        }
    }
    false
}

/// Whether a wall, doorway post, window, frame rim or diagonal wall stops
/// the horizontal move (x,z)→(nx,nz) at `feet_y`. Movement steps are
/// ≤ 0.19 m, so testing the endpoint's z against the edge span (instead
/// of the exact crossing point) cuts at most a fingertip off a post
/// corner.
pub fn blocked(seed: u64, cols: &ColIndex, x: f32, z: f32, nx: f32, nz: f32, feet_y: f32) -> bool {
    let (bx0, bz0) = (
        crate::build::build_cell_of(x),
        crate::build::build_cell_of(z),
    );
    let (bx1, bz1) = (
        crate::build::build_cell_of(nx),
        crate::build::build_cell_of(nz),
    );
    if cell_edges_block(seed, cols, bx1, bz1, x, z, nx, nz, feet_y) {
        return true;
    }
    if (bx0 != bx1 || bz0 != bz1) && cell_edges_block(seed, cols, bx0, bz0, x, z, nx, nz, feet_y) {
        return true;
    }
    let (lo, hi) = (feet_y, feet_y + CAPSULE_HEIGHT_M);
    if cell_diags_block(
        seed,
        cols,
        bx1,
        bz1,
        x,
        z,
        nx,
        nz,
        lo,
        hi,
        false,
        CAPSULE_RADIUS_M,
    ) {
        return true;
    }
    if (bx0 != bx1 || bz0 != bz1)
        && cell_diags_block(
            seed,
            cols,
            bx0,
            bz0,
            x,
            z,
            nx,
            nz,
            lo,
            hi,
            false,
            CAPSULE_RADIUS_M,
        )
    {
        return true;
    }
    false
}

/// Whether an edge piece stops a **shot** sample moving (x,z)→(nx,nz) at
/// altitude `y` with radius `r` — [`blocked`]'s walk with a projectile's
/// profile instead of a body's. Three differences, each the point:
///
/// - the mover is a point at `y`, not a 1.7 m capsule standing on `feet_y`
///   — an arrow meets the one storey its altitude is inside;
/// - the slab is inflated by `r` (the arrowhead), not `CAPSULE_RADIUS_M`,
///   so a shot threads what a body cannot — the honest fix `ranged.rs`'s
///   module doc owed since ranged v0;
/// - a **window is solid everywhere but its aperture**
///   ([`window_solid_at`]), where the body walk treats it as a wall; a
///   **frame** is solid only at its drawn rim ([`frame_solid_at`]); an
///   open **doorway** stops a shot at its posts *and* its lintel
///   ([`DOOR_HEAD_M`]), where a body only ever met the posts; and a
///   **shut door** seals its doorway on both walks.
#[allow(clippy::too_many_arguments)]
pub fn shot_blocked(
    seed: u64,
    cols: &ColIndex,
    x: f32,
    z: f32,
    nx: f32,
    nz: f32,
    y: f32,
    r: f32,
) -> bool {
    let (bx0, bz0) = (
        crate::build::build_cell_of(x),
        crate::build::build_cell_of(z),
    );
    let (bx1, bz1) = (
        crate::build::build_cell_of(nx),
        crate::build::build_cell_of(nz),
    );
    if cell_edges_stop_shot(seed, cols, bx1, bz1, x, z, nx, nz, y, r) {
        return true;
    }
    if (bx0 != bx1 || bz0 != bz1) && cell_edges_stop_shot(seed, cols, bx0, bz0, x, z, nx, nz, y, r)
    {
        return true;
    }
    // The diagonals, with the point's own band: containment of `y`, not a
    // capsule's overlap.
    if cell_diags_block(seed, cols, bx1, bz1, x, z, nx, nz, y, y, true, r) {
        return true;
    }
    if (bx0 != bx1 || bz0 != bz1)
        && cell_diags_block(seed, cols, bx0, bz0, x, z, nx, nz, y, y, true, r)
    {
        return true;
    }
    false
}

/// [`cell_edges_block`] with the shot profile — the four boundaries of one
/// cell against a point sample. Kept beside its body twin so the two walks
/// read as one algorithm with two movers, which they are.
#[allow(clippy::too_many_arguments)]
fn cell_edges_stop_shot(
    seed: u64,
    cols: &ColIndex,
    bx: i32,
    bz: i32,
    x: f32,
    z: f32,
    nx: f32,
    nz: f32,
    y: f32,
    r: f32,
) -> bool {
    let edges = [
        (bx, bz, true),
        (bx + 1, bz, true),
        (bx, bz, false),
        (bx, bz + 1, false),
    ];
    for (ecx, ecz, x_plane) in edges {
        if ecx < 0 || ecz < 0 || ecx >= MAX_BUILD_COORD as i32 || ecz >= MAX_BUILD_COORD as i32 {
            continue;
        }
        let m = cols.get(ecx as u16, ecz as u16);
        let (walls, doors, wins, frames, shuts) = if x_plane {
            (
                m.walls_xlo,
                m.doors_xlo,
                m.wins_xlo,
                m.frames_xlo,
                m.shut_xlo,
            )
        } else {
            (
                m.walls_zlo,
                m.doors_zlo,
                m.wins_zlo,
                m.frames_zlo,
                m.shut_zlo,
            )
        };
        if walls | doors | wins | frames == 0 {
            continue;
        }
        let base = col_base_y(seed, ecx as u16, ecz as u16);
        for level in 0..MAX_BUILD_LEVELS {
            let bit = 1u8 << level;
            if (walls | doors | wins | frames) & bit == 0 {
                continue;
            }
            let bottom = base + level as f32 * LEVEL_H_M;
            // A point is inside exactly one storey; half-open so the
            // boundary altitude resolves the same on both targets.
            if y < bottom || y >= bottom + LEVEL_H_M {
                continue;
            }
            let meet = if x_plane {
                let px = ecx as f32 * BUILD_CELL_M;
                let s0 = ecz as f32 * BUILD_CELL_M;
                edge_meet(px, s0, x, nx, nz, r)
            } else {
                let pz = ecz as f32 * BUILD_CELL_M;
                let s0 = ecx as f32 * BUILD_CELL_M;
                edge_meet(pz, s0, z, nz, nx, r)
            };
            let Some(t) = meet else { continue };
            let span = (0.0..=BUILD_CELL_M).contains(&t);
            let solid = if walls & bit != 0 {
                span
            } else if doors & bit != 0 {
                if shuts & bit != 0 {
                    span // a closed door seals the doorway for shots too
                } else {
                    // Posts, and the lintel a body never had to answer to:
                    // the drawn 2.1..3.0 band stops an arrow now
                    // (`DOOR_HEAD_M`'s doc).
                    span && (doorway_solid_at(t) || y - bottom >= DOOR_HEAD_M)
                }
            } else if wins & bit != 0 {
                span && window_solid_at(t, y - bottom)
            } else {
                span && frame_solid_at(t, y - bottom)
            };
            if solid {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build::{
        place, BuildContent, PieceDef, Pieces, LOC_PLANE, LOC_RISER, MAT_TWIG, SHAPE_DOORWAY,
        SHAPE_STAIRS,
    };
    use crate::deploy::Deploys;
    use crate::input::InputFrame;
    use crate::movement::{self, Body, POS_XZ_Q, POS_Y_Q};
    use crate::rng::Pcg32;
    use crate::terrain;
    use crate::world::{EventQueue, Player, EV_PIECE_PLACED};

    /// The browser-smoke seed and its guarded-walkable cell (build.rs
    /// tests use the same anchors).
    const SEED: u64 = 20260731;
    const CX: u16 = 341;
    const CZ: u16 = 341;

    /// Free pieces (n_costs 0) so tests place without inventories: rows
    /// foundation, wall, doorway, floor, stairs, window, frame. **Twig**,
    /// because `put` goes through the real `place` verb and a placement is
    /// twig or it is refused (twig v0) — collision is a property of the
    /// shape, so the rung these carry has never been what these tests are
    /// about.
    fn free_table() -> BuildContent {
        let shapes = [
            SHAPE_FOUNDATION,
            SHAPE_WALL,
            SHAPE_DOORWAY,
            SHAPE_FLOOR,
            SHAPE_STAIRS,
            SHAPE_WINDOW,
            SHAPE_FRAME,
            SHAPE_TRI_FOUNDATION,
        ];
        let mut b = BuildContent::EMPTY;
        b.piece_count = shapes.len() as u16;
        for (i, &shape) in shapes.iter().enumerate() {
            b.pieces[i] = PieceDef {
                shape,
                material: MAT_TWIG,
                hp: 100,
                n_costs: 0,
                costs: [(0, 0); crate::limits::MAX_PIECE_COSTS],
            };
        }
        b
    }

    /// Place row `row` at the address through the real verb; panics on a
    /// refusal so a broken fixture can't silently pass a collision test.
    fn put(bc: &BuildContent, pieces: &mut Pieces, cx: u16, cz: u16, level: u8, loc: u8, row: u16) {
        let mut p = Player {
            id: 9,
            active: true,
            body: Body::at(
                SEED,
                (cx as f32 + 0.5) * BUILD_CELL_M,
                (cz as f32 + 0.5) * BUILD_CELL_M,
            ),
            ..Player::default()
        };
        let mut ev = EventQueue::default();
        let nod = Deploys::new();
        place(
            SEED, bc, &nod, pieces, &mut p, 0, row, cx, cz, level, loc, &mut ev,
        );
        let last = ev.entries()[ev.len() - 1];
        assert_eq!(
            last.code, EV_PIECE_PLACED,
            "fixture place refused: row {row} at ({cx},{cz},{level},{loc}) reason {}",
            last.b
        );
    }

    fn body_at(x: f32, z: f32) -> Body {
        Body::at(SEED, x, z)
    }

    fn walk(frame_x: i8, frame_z: i8) -> InputFrame {
        InputFrame {
            seq: 1,
            buttons: 0,
            yaw: 0, // forward +Z, right +X
            pitch: 0,
            move_x: frame_x,
            move_z: frame_z,
            sel: 0,
        }
    }

    fn pos(b: &Body) -> (f32, f32, f32) {
        (
            b.qx as f32 * POS_XZ_Q,
            b.qy as f32 * POS_Y_Q,
            b.qz as f32 * POS_XZ_Q,
        )
    }

    #[test]
    fn wall_blocks_doorway_opens_posts_block() {
        // Pieces are what this fixture is about; a pine standing where it
        // walks is not (occupy::Barren).
        let mut occ = crate::occupy::Scratch::barren();
        let bc = free_table();
        let wall_x = CX as f32 * BUILD_CELL_M; // 1023: the low-x edge plane

        // A wall on the low-x edge stops a −x walk at the slab.
        let mut pieces = Pieces::new();
        put(&bc, &mut pieces, CX, CZ, 0, LOC_PLANE, 0);
        put(&bc, &mut pieces, CX, CZ, 0, crate::build::LOC_EDGE_XLO, 1);
        let mut b = body_at(1024.5, 1024.5);
        for _ in 0..120 {
            movement::step(
                SEED,
                pieces.cols(),
                &mut occ.occupants(),
                &mut b,
                &walk(-127, 0),
            );
        }
        let (x, _, _) = pos(&b);
        let r = WALL_THICKNESS_M * 0.5 + CAPSULE_RADIUS_M;
        assert!(
            x >= wall_x + r - POS_XZ_Q,
            "wall failed to block: x {x} < plane {wall_x} + r {r}"
        );
        assert!(x < 1024.0, "the walk never approached the wall");

        // Same walk, doorway instead: the centered opening passes.
        let mut pieces = Pieces::new();
        put(&bc, &mut pieces, CX, CZ, 0, LOC_PLANE, 0);
        put(&bc, &mut pieces, CX, CZ, 0, crate::build::LOC_EDGE_XLO, 2);
        let mut b = body_at(1024.5, 1024.5);
        for _ in 0..120 {
            movement::step(
                SEED,
                pieces.cols(),
                &mut occ.occupants(),
                &mut b,
                &walk(-127, 0),
            );
        }
        assert!(
            pos(&b).0 < wall_x - 0.5,
            "doorway opening should pass: x {}",
            pos(&b).0
        );

        // Aimed at a post (z inside the low-z post span): blocked.
        let mut b = body_at(1024.5, CZ as f32 * BUILD_CELL_M + 0.45);
        for _ in 0..120 {
            movement::step(
                SEED,
                pieces.cols(),
                &mut occ.occupants(),
                &mut b,
                &walk(-127, 0),
            );
        }
        assert!(
            pos(&b).0 >= wall_x + r - POS_XZ_Q,
            "doorway post failed to block: x {}",
            pos(&b).0
        );

        // A wall one storey up over an open edge is above the head:
        // foundation + low-z wall carry a level-1 floor, which supports a
        // level-1 low-x wall over a bare level-0 low-x edge — walking out
        // along −x at ground level passes under it.
        let mut pieces = Pieces::new();
        put(&bc, &mut pieces, CX, CZ, 0, LOC_PLANE, 0);
        put(&bc, &mut pieces, CX, CZ, 0, crate::build::LOC_EDGE_ZLO, 1);
        put(&bc, &mut pieces, CX, CZ, 1, LOC_PLANE, 3);
        put(&bc, &mut pieces, CX, CZ, 1, crate::build::LOC_EDGE_XLO, 1);
        let mut b = body_at(1024.5, 1024.5);
        for _ in 0..120 {
            movement::step(
                SEED,
                pieces.cols(),
                &mut occ.occupants(),
                &mut b,
                &walk(-127, 0),
            );
        }
        assert!(
            pos(&b).0 < wall_x - 0.5,
            "a level-1 wall must not block ground movement: x {}",
            pos(&b).0
        );
    }

    #[test]
    fn planes_are_ground_and_edges_drop_off() {
        // Pieces are what this fixture is about; a pine standing where it
        // walks is not (occupy::Barren).
        let mut occ = crate::occupy::Scratch::barren();
        let bc = free_table();
        let mut pieces = Pieces::new();
        put(&bc, &mut pieces, CX, CZ, 0, LOC_PLANE, 0);
        let base = col_base_y(SEED, CX, CZ);

        // Standing in the cell snaps up onto the slab (lift ≤ step-up)…
        let mut b = body_at(1024.5, 1024.5);
        movement::step(
            SEED,
            pieces.cols(),
            &mut occ.occupants(),
            &mut b,
            &walk(0, 0),
        );
        let (_, y, _) = pos(&b);
        assert!(
            fabs(y - base) <= POS_Y_Q,
            "feet {y} should sit on the foundation top {base}"
        );

        // …a floor two storeys up is a ceiling, not a teleport.
        put(&bc, &mut pieces, CX, CZ, 0, crate::build::LOC_EDGE_XLO, 1);
        put(&bc, &mut pieces, CX, CZ, 1, crate::build::LOC_EDGE_XLO, 1);
        put(&bc, &mut pieces, CX, CZ, 2, LOC_PLANE, 3);
        movement::step(
            SEED,
            pieces.cols(),
            &mut occ.occupants(),
            &mut b,
            &walk(0, 0),
        );
        let (_, y, _) = pos(&b);
        assert!(
            fabs(y - base) <= POS_Y_Q,
            "feet {y} teleported toward the level-2 floor"
        );

        // Walking +x off the slab falls back to terrain.
        let mut b = body_at(1024.5, 1024.5);
        for _ in 0..240 {
            movement::step(
                SEED,
                pieces.cols(),
                &mut occ.occupants(),
                &mut b,
                &walk(127, 0),
            );
        }
        let (x, y, z) = pos(&b);
        let terr = terrain::height(SEED, x, z);
        assert!(x > (CX + 1) as f32 * BUILD_CELL_M, "never left the cell");
        assert!(
            b.grounded && fabs(y - terr) <= STEP_UP,
            "feet {y} should be back on terrain {terr}"
        );
    }

    #[test]
    fn stairs_ramp_climbs_the_storey() {
        // Pieces are what this fixture is about; a pine standing where it
        // walks is not (occupy::Barren).
        let mut occ = crate::occupy::Scratch::barren();
        let bc = free_table();
        let mut pieces = Pieces::new();
        put(&bc, &mut pieces, CX, CZ, 0, LOC_PLANE, 0);
        put(&bc, &mut pieces, CX, CZ, 0, LOC_RISER, 4);
        let base = col_base_y(SEED, CX, CZ);

        // Walk +Z up the ramp: feet rise monotonically to base + storey.
        let mut b = body_at(1024.5, CZ as f32 * BUILD_CELL_M + 0.2);
        let mut last_y = pos(&b).1;
        let mut top_y = last_y;
        for _ in 0..180 {
            movement::step(
                SEED,
                pieces.cols(),
                &mut occ.occupants(),
                &mut b,
                &walk(0, 127),
            );
            let (_, y, z) = pos(&b);
            if z < (CZ + 1) as f32 * BUILD_CELL_M {
                assert!(y >= last_y - POS_Y_Q, "ramp descended while walking up");
                last_y = y;
                top_y = y;
            }
        }
        assert!(
            fabs(top_y - (base + LEVEL_H_M)) <= STEP_UP,
            "ramp top {top_y} should reach the next storey {}",
            base + LEVEL_H_M
        );
        // And past the cell it drops back to terrain (nothing up there).
        let (x, y, z) = pos(&b);
        let terr = terrain::height(SEED, x, z);
        assert!(z > (CZ + 1) as f32 * BUILD_CELL_M, "never crested the ramp");
        assert!(
            b.grounded && fabs(y - terr) <= STEP_UP,
            "feet {y} should have fallen back to terrain {terr}"
        );
    }

    /// The catalogue-v1 collision contract, all three clauses: a window
    /// blocks a body exactly as a wall does, a frame blocks nothing, and
    /// the shot walk passes the window's aperture while stopping on its
    /// sill, jambs and the wall beside it.
    #[test]
    fn window_blocks_a_body_and_not_a_shot_and_a_frame_blocks_neither() {
        let mut occ = crate::occupy::Scratch::barren();
        let bc = free_table();
        let wall_x = CX as f32 * BUILD_CELL_M;
        let r = WALL_THICKNESS_M * 0.5 + CAPSULE_RADIUS_M;

        // A window on the x_plane edge stops a westward walk at the slab —
        // the aimed line passes the aperture's metres, so what stops the
        // body is the window being a wall to a capsule, not a jamb.
        let mut pieces = Pieces::new();
        put(&bc, &mut pieces, CX, CZ, 0, LOC_PLANE, 0);
        put(&bc, &mut pieces, CX, CZ, 0, crate::build::LOC_EDGE_XLO, 5);
        let mut b = body_at(1024.5, 1024.5);
        for _ in 0..120 {
            movement::step(
                SEED,
                pieces.cols(),
                &mut occ.occupants(),
                &mut b,
                &walk(-127, 0),
            );
        }
        assert!(
            pos(&b).0 >= wall_x + r - POS_XZ_Q,
            "window failed to block a body: x {}",
            pos(&b).0
        );

        // The same edge, to a shot: the storey base is the column's own.
        let base = col_base_y(SEED, CX, CZ);
        let (z_open, z_jamb) = (
            CZ as f32 * BUILD_CELL_M + BUILD_CELL_M * 0.5, // mid-opening
            CZ as f32 * BUILD_CELL_M + 0.4,                // inside a jamb
        );
        let (x0, x1) = (wall_x + 0.5, wall_x - 0.5);
        let mid_band = base + (WINDOW_SILL_M + WINDOW_HEAD_M) * 0.5;
        assert!(
            !shot_blocked(SEED, pieces.cols(), x0, z_open, x1, z_open, mid_band, 0.05),
            "an arrow through the aperture must pass"
        );
        assert!(
            shot_blocked(
                SEED,
                pieces.cols(),
                x0,
                z_open,
                x1,
                z_open,
                base + 0.5,
                0.05
            ),
            "the sill under the aperture must stop it"
        );
        assert!(
            shot_blocked(
                SEED,
                pieces.cols(),
                x0,
                z_open,
                x1,
                z_open,
                base + WINDOW_HEAD_M + 0.3,
                0.05
            ),
            "the header over the aperture must stop it"
        );
        assert!(
            shot_blocked(SEED, pieces.cols(), x0, z_jamb, x1, z_jamb, mid_band, 0.05),
            "a jamb must stop it at aperture height"
        );

        // A wall stops the same shot everywhere; sanity beside the window.
        let mut walled = Pieces::new();
        put(&bc, &mut walled, CX, CZ, 0, LOC_PLANE, 0);
        put(&bc, &mut walled, CX, CZ, 0, crate::build::LOC_EDGE_XLO, 1);
        assert!(
            shot_blocked(SEED, walled.cols(), x0, z_open, x1, z_open, mid_band, 0.05),
            "a wall stops what a window's aperture passes"
        );

        // A frame passes both movers through its opening — only the thin
        // drawn rim is solid, to bodies and shots alike.
        let mut framed = Pieces::new();
        put(&bc, &mut framed, CX, CZ, 0, LOC_PLANE, 0);
        put(&bc, &mut framed, CX, CZ, 0, crate::build::LOC_EDGE_XLO, 6);
        let mut b = body_at(1024.5, 1024.5);
        for _ in 0..120 {
            movement::step(
                SEED,
                framed.cols(),
                &mut occ.occupants(),
                &mut b,
                &walk(-127, 0),
            );
        }
        assert!(
            pos(&b).0 < wall_x - 0.5,
            "an empty frame must pass a body through its opening: x {}",
            pos(&b).0
        );
        assert!(
            !shot_blocked(
                SEED,
                framed.cols(),
                x0,
                z_jamb,
                x1,
                z_jamb,
                base + 0.5,
                0.05
            ),
            "an empty frame must pass a shot through its opening"
        );
        let z_rim = CZ as f32 * BUILD_CELL_M + 0.05;
        assert!(
            shot_blocked(SEED, framed.cols(), x0, z_rim, x1, z_rim, base + 0.5, 0.05),
            "the frame's rim jamb must stop a shot"
        );
        assert!(
            shot_blocked(
                SEED,
                framed.cols(),
                x0,
                z_open,
                x1,
                z_open,
                base + LEVEL_H_M - 0.05,
                0.05
            ),
            "the frame's top beam must stop a shot"
        );

        // The doorway's lintel finally answers to something: an arrow
        // through the drawn 2.1..3.0 band stops, one through the opening
        // does not (`DOOR_HEAD_M`).
        let mut doored = Pieces::new();
        put(&bc, &mut doored, CX, CZ, 0, LOC_PLANE, 0);
        put(&bc, &mut doored, CX, CZ, 0, crate::build::LOC_EDGE_XLO, 2);
        assert!(
            !shot_blocked(
                SEED,
                doored.cols(),
                x0,
                z_open,
                x1,
                z_open,
                base + 1.5,
                0.05
            ),
            "the doorway opening passes a shot"
        );
        assert!(
            shot_blocked(
                SEED,
                doored.cols(),
                x0,
                z_open,
                x1,
                z_open,
                base + DOOR_HEAD_M + 0.3,
                0.05
            ),
            "the doorway lintel stops a shot"
        );
    }

    /// Triangles v0's collision contract: a tri foundation is ground over
    /// its own half and terrain over the other, and a diagonal wall stops
    /// a walk and a shot across its line while the open half-cell passes.
    #[test]
    fn tri_ground_holds_its_half_and_the_diagonal_blocks_the_cross() {
        let mut occ = crate::occupy::Scratch::barren();
        let bc = free_table();
        let mut pieces = Pieces::new();
        // The NW half (dx + dz <= 3) and its hypotenuse wall on B.
        put(
            &bc,
            &mut pieces,
            CX,
            CZ,
            0,
            crate::build::LOC_TRI_XLO_ZLO,
            7,
        );
        let base = col_base_y(SEED, CX, CZ);
        let x0 = CX as f32 * BUILD_CELL_M;
        let z0 = CZ as f32 * BUILD_CELL_M;

        // Ground on the NW side of the split; terrain on the SE side.
        let on = piece_ground(SEED, pieces.cols(), x0 + 0.7, z0 + 0.7, base + 0.05);
        assert!(
            fabs(on - base) < 1e-4,
            "the half's own ground reads the slab: {on} vs {base}"
        );
        let off = piece_ground(SEED, pieces.cols(), x0 + 2.5, z0 + 2.5, base + 0.05);
        assert_eq!(off, NO_SURFACE, "the other half is not this piece");

        // The diagonal wall across B: a walk from the NW half toward SE
        // stops at the line; with no wall it passes.
        let mut b = body_at(x0 + 1.0, z0 + 1.0);
        for _ in 0..160 {
            movement::step(
                SEED,
                pieces.cols(),
                &mut occ.occupants(),
                &mut b,
                &walk(90, 90),
            );
        }
        assert!(
            pos(&b).0 > x0 + 2.5 && pos(&b).2 > z0 + 2.5,
            "with no diagonal wall the cross passes: ({}, {})",
            pos(&b).0,
            pos(&b).2
        );
        put(&bc, &mut pieces, CX, CZ, 0, crate::build::LOC_DIAG_B, 1);
        let mut b = body_at(x0 + 1.0, z0 + 1.0);
        for _ in 0..160 {
            movement::step(
                SEED,
                pieces.cols(),
                &mut occ.occupants(),
                &mut b,
                &walk(90, 90),
            );
        }
        let (bx, _, bz) = pos(&b);
        assert!(
            (bx - x0) + (bz - z0) < BUILD_CELL_M,
            "the diagonal wall must hold the walker on its own side: ({bx}, {bz})"
        );

        // The shot walk agrees: across the line stops, along the open
        // half passes.
        assert!(
            shot_blocked(
                SEED,
                pieces.cols(),
                x0 + 1.0,
                z0 + 1.0,
                x0 + 2.2,
                z0 + 2.2,
                base + 1.5,
                0.05
            ),
            "an arrow across the diagonal stops"
        );
        assert!(
            !shot_blocked(
                SEED,
                pieces.cols(),
                x0 + 0.3,
                z0 + 1.0,
                x0 + 1.0,
                z0 + 0.3,
                base + 1.5,
                0.05
            ),
            "an arrow inside the NW half never meets the line"
        );
    }

    #[test]
    fn col_index_churn_matches_a_naive_shadow() {
        // Random add/del churn over a small key space, checked against a
        // Vec shadow after every op — this is what pins backward-shift
        // deletion (test code may allocate; the index itself never does).
        let mut idx = Box::new(ColIndex::new());
        let mut shadow: Vec<(u16, u16, u8, u8, u8)> = Vec::new();
        let mut rng = Pcg32::new(0x0C01_11DE, 5);
        let shapes = [
            SHAPE_FOUNDATION,
            SHAPE_WALL,
            SHAPE_DOORWAY,
            SHAPE_FLOOR,
            SHAPE_STAIRS,
            SHAPE_WINDOW,
            SHAPE_FRAME,
        ];
        for _ in 0..20_000 {
            let cx = rng.next_bounded(24) as u16;
            let cz = rng.next_bounded(24) as u16;
            let level = rng.next_bounded(8) as u8;
            let shape = shapes[rng.next_bounded(7) as usize];
            let loc = match shape {
                SHAPE_WALL | SHAPE_DOORWAY | SHAPE_WINDOW | SHAPE_FRAME => {
                    (2 + rng.next_bounded(2)) as u8
                }
                SHAPE_STAIRS => LOC_RISER,
                _ => LOC_PLANE,
            };
            // One piece per address — the store's invariant (place()
            // refuses occupied addresses), so a del always names the
            // shape that was inserted there.
            let at = shadow
                .iter()
                .position(|e| e.0 == cx && e.1 == cz && e.2 == level && e.3 == loc);
            match at {
                None if rng.next_bounded(5) < 3 => {
                    idx.add(cx, cz, level, loc, shape);
                    shadow.push((cx, cz, level, loc, shape));
                }
                Some(i) if rng.next_bounded(5) >= 3 => {
                    let (_, _, _, _, stored) = shadow.swap_remove(i);
                    idx.del(cx, cz, level, loc, stored);
                }
                _ => {}
            }
        }
        // Every column the shadow can name must agree bit for bit.
        for cx in 0..24u16 {
            for cz in 0..24u16 {
                let mut want = ColMasks::EMPTY;
                for &(sx, sz, level, loc, shape) in &shadow {
                    if sx == cx && sz == cz {
                        if let Some(m) = want.field(shape, loc) {
                            *m |= 1 << level;
                        }
                    }
                }
                assert_eq!(idx.get(cx, cz), want, "column ({cx},{cz}) drifted");
            }
        }
        // Drain the shadow: the table must come back perfectly empty.
        for &(cx, cz, level, loc, shape) in shadow.clone().iter() {
            idx.del(cx, cz, level, loc, shape);
        }
        assert_eq!(idx.len(), 0);
        assert!(idx.keys.iter().all(|&k| k == 0), "a slot leaked");
    }

    #[test]
    fn decay_removal_clears_the_column() {
        // Pieces are what this fixture is about; a pine standing where it
        // walks is not (occupy::Barren).
        let mut occ = crate::occupy::Scratch::barren();
        let bc = free_table();
        let mut pieces = Pieces::new();
        put(&bc, &mut pieces, CX, CZ, 0, LOC_PLANE, 0);
        put(&bc, &mut pieces, CX, CZ, 0, crate::build::LOC_EDGE_XLO, 1);
        assert_eq!(pieces.cols().get(CX, CZ).walls_xlo, 1);
        // The sweep's removal path: drop the wall by store index.
        let wi = pieces
            .entries()
            .iter()
            .position(|r| r.loc == crate::build::LOC_EDGE_XLO)
            .unwrap();
        pieces.remove_at(wi, SHAPE_WALL);
        assert_eq!(pieces.cols().get(CX, CZ).walls_xlo, 0);
        assert_eq!(pieces.cols().get(CX, CZ).planes, 1, "the slab stays");
        let mut b = body_at(1024.5, 1024.5);
        for _ in 0..120 {
            movement::step(
                SEED,
                pieces.cols(),
                &mut occ.occupants(),
                &mut b,
                &walk(-127, 0),
            );
        }
        assert!(
            pos(&b).0 < CX as f32 * BUILD_CELL_M - 0.5,
            "a decayed wall must stop blocking"
        );
    }
}
