//! Piece collision v0 (TERRAIN.md §3 "Buildings: AABB/oriented boxes per
//! block"): placed building pieces become movement collision, shared
//! verbatim by the server tick and the wasm predictor — the client's
//! mirror of the piece store feeds the exact code the server runs, so
//! prediction through a doorway holds bit for bit (skew is bounded by the
//! one in-flight placement event, the same bound the slot store accepts).
//!
//! Geometry: a piece's vertical base is `terrain::height(cell center) +
//! PIECE_LIFT_M + level·LEVEL_H_M` — the renderer's formula (scene.js),
//! now sim-authoritative. Planes (foundation/floor/roof) are walkable
//! surfaces at their base; stairs are a ramp rising toward +Z through the
//! storey; walls block their edge for the storey they span; doorways
//! block only their posts (the 1.2 m opening passes; the lintel never
//! matters at capsule height until a jump exists). A **closed door**
//! deployable in a doorway blocks the whole edge like a wall; open doors
//! and empty doorways pass. Door state is a shut bit per (column, level,
//! edge), maintained by deploy.rs in lockstep with the door records —
//! derived state like the rest of the index.
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
    BUILD_CELL_M, LEVEL_H_M, LOC_EDGE_N, LOC_EDGE_W, SHAPE_DOORWAY, SHAPE_FLOOR, SHAPE_FOUNDATION,
    SHAPE_ROOF, SHAPE_STAIRS, SHAPE_WALL,
};
use crate::fmath::fabs;
use crate::limits::{COL_INDEX_SLOTS, MAX_BUILD_COORD, MAX_BUILD_LEVELS};
use crate::movement::STEP_UP;
use crate::terrain;

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

/// "No built surface here" sentinel — far below any terrain, so
/// `max(terrain, piece_ground)` needs no branch.
pub const NO_SURFACE: f32 = -1.0e9;

/// Per-column occupancy, one bit per level (MAX_BUILD_LEVELS = 8 fits u8
/// exactly). Edge masks live in their canonical column (build.rs: west/
/// north), so a cell's east edge is its +x neighbor's `*_w`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ColMasks {
    pub planes: u8,
    pub stairs: u8,
    pub walls_w: u8,
    pub walls_n: u8,
    pub doors_w: u8,
    pub doors_n: u8,
    /// Closed-door bits: set ⇒ the doorway at this level/edge holds a
    /// closed door deployable and blocks its full span (deploy.rs keeps
    /// these in lockstep with the door records).
    pub shut_w: u8,
    pub shut_n: u8,
}

impl ColMasks {
    pub const EMPTY: Self = Self {
        planes: 0,
        stairs: 0,
        walls_w: 0,
        walls_n: 0,
        doors_w: 0,
        doors_n: 0,
        shut_w: 0,
        shut_n: 0,
    };

    fn is_empty(&self) -> bool {
        (self.planes
            | self.stairs
            | self.walls_w
            | self.walls_n
            | self.doors_w
            | self.doors_n
            | self.shut_w
            | self.shut_n)
            == 0
    }

    /// The mask a (shape, loc) pair lives in, or None for shapes with no
    /// collision footprint.
    fn field(&mut self, shape: u8, loc: u8) -> Option<&mut u8> {
        match shape {
            SHAPE_FOUNDATION | SHAPE_FLOOR | SHAPE_ROOF => Some(&mut self.planes),
            SHAPE_STAIRS => Some(&mut self.stairs),
            SHAPE_WALL if loc == LOC_EDGE_W => Some(&mut self.walls_w),
            SHAPE_WALL if loc == LOC_EDGE_N => Some(&mut self.walls_n),
            SHAPE_DOORWAY if loc == LOC_EDGE_W => Some(&mut self.doors_w),
            SHAPE_DOORWAY if loc == LOC_EDGE_N => Some(&mut self.doors_n),
            _ => None,
        }
    }
}

/// Open-addressed column map, linear probing, backward-shift deletion —
/// no tombstones, so a shard that builds and decays for months never
/// degrades. Fixed capacity (limits.rs `COL_INDEX_SLOTS` = 2 × the piece
/// cap), keys packed `1<<31 | cx<<10 | cz` so 0 means empty.
pub struct ColIndex {
    keys: [u32; COL_INDEX_SLOTS],
    masks: [ColMasks; COL_INDEX_SLOTS],
    len: u32,
}

const OCCUPIED: u32 = 1 << 31;

impl ColIndex {
    pub fn new() -> Self {
        Self {
            keys: [0; COL_INDEX_SLOTS],
            masks: [ColMasks::EMPTY; COL_INDEX_SLOTS],
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
        self.keys = [0; COL_INDEX_SLOTS];
        self.masks = [ColMasks::EMPTY; COL_INDEX_SLOTS];
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
        if loc != LOC_EDGE_W && loc != LOC_EDGE_N {
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
            if loc == LOC_EDGE_W {
                m.shut_w &= !(1 << level);
            } else {
                m.shut_n &= !(1 << level);
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
        if loc == LOC_EDGE_W {
            m.shut_w |= 1 << level;
        } else {
            m.shut_n |= 1 << level;
        }
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

/// A column's level-0 base: the renderer's `groundY + LIFT` (scene.js),
/// sampled at the cell center — no piece height ever rides the wire.
#[inline]
pub(crate) fn col_base_y(seed: u64, cx: u16, cz: u16) -> f32 {
    let half = BUILD_CELL_M * 0.5;
    terrain::height(
        seed,
        cx as f32 * BUILD_CELL_M + half,
        cz as f32 * BUILD_CELL_M + half,
    ) + PIECE_LIFT_M
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
    if m.planes == 0 && m.stairs == 0 {
        return NO_SURFACE;
    }
    let base = col_base_y(seed, bx as u16, bz as u16);
    let lid = feet_y + STEP_UP;
    let mut best = NO_SURFACE;
    for level in 0..MAX_BUILD_LEVELS {
        let bit = 1u8 << level;
        let floor = base + level as f32 * LEVEL_H_M;
        if m.planes & bit != 0 && floor <= lid && floor > best {
            best = floor;
        }
        if m.stairs & bit != 0 {
            // The ramp rises toward +Z through the storey (scene.js).
            let frac = ((z - bz as f32 * BUILD_CELL_M) / BUILD_CELL_M).clamp(0.0, 1.0);
            let ramp = floor + frac * LEVEL_H_M;
            if ramp <= lid && ramp > best {
                best = ramp;
            }
        }
    }
    best
}

/// One edge's block test: does the move (x,z)→(nx,nz) cross or push into
/// the inflated edge slab, at a z the edge actually occupies? `posts`
/// selects doorway geometry (only the posts block). `axis_w` says the
/// edge is an x-plane (west) rather than a z-plane (north); coordinates
/// are pre-swapped by the caller so the math is written once.
fn edge_hit(px: f32, s0: f32, a0: f32, a1: f32, along: f32, posts: bool) -> bool {
    let r = WALL_THICKNESS_M * 0.5 + CAPSULE_RADIUS_M;
    let d0 = a0 - px;
    let d1 = a1 - px;
    let crosses = (d0 < 0.0 && d1 > 0.0) || (d0 > 0.0 && d1 < 0.0);
    let pushes_in = fabs(d1) < r && fabs(d1) < fabs(d0);
    if !crosses && !pushes_in {
        return false;
    }
    let t = along - s0;
    if posts {
        doorway_solid_at(t)
    } else {
        (0.0..=BUILD_CELL_M).contains(&t)
    }
}

/// Is a doorway SOLID at `t` metres along its edge? The two posts block; the
/// gap between them is the opening.
///
/// **Extracted so the renderer can be gated against it rather than against a
/// copy of it.** `render/ghost.rs` and `render/structures.rs` both draw this
/// doorway, and `crates/client/tests/ghost.rs` asserts the drawn posts land
/// exactly where this returns true. `RENDER.md` §8 states the stake: the
/// opening is what `edge_hit` refuses to block, and "draw it elsewhere and the
/// frame lies about where a player can walk". A test that restated this band
/// instead of calling it would go green while the two drifted, which is the
/// byte-golden hole `CLAUDE.md`'s trap list names.
///
/// Pure, allocation-free, comparison-only — wall 1 is untouched.
pub fn doorway_solid_at(t: f32) -> bool {
    (0.0..=DOOR_POST_W_M).contains(&t) || (BUILD_CELL_M - DOOR_POST_W_M..=BUILD_CELL_M).contains(&t)
}

/// All blocking edges of one cell against the move. The four edges are
/// the cell's canonical west/north plus the neighbors' (build.rs edge
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
    // (column cx, cz, west-edge?) — the four boundaries of cell (bx, bz).
    let edges = [
        (bx, bz, true),
        (bx + 1, bz, true),
        (bx, bz, false),
        (bx, bz + 1, false),
    ];
    for (ecx, ecz, west) in edges {
        if ecx < 0 || ecz < 0 || ecx >= MAX_BUILD_COORD as i32 || ecz >= MAX_BUILD_COORD as i32 {
            continue;
        }
        let m = cols.get(ecx as u16, ecz as u16);
        let (walls, doors, shuts) = if west {
            (m.walls_w, m.doors_w, m.shut_w)
        } else {
            (m.walls_n, m.doors_n, m.shut_n)
        };
        if walls | doors == 0 {
            continue;
        }
        let base = col_base_y(seed, ecx as u16, ecz as u16);
        for level in 0..MAX_BUILD_LEVELS {
            let bit = 1u8 << level;
            let has_wall = walls & bit != 0;
            let has_door = doors & bit != 0;
            if !has_wall && !has_door {
                continue;
            }
            // A closed door seals the doorway: full span, like a wall.
            let posts_only = has_door && !has_wall && shuts & bit == 0;
            let bottom = base + level as f32 * LEVEL_H_M;
            if feet_y >= bottom + LEVEL_H_M || feet_y + CAPSULE_HEIGHT_M <= bottom {
                continue; // that storey is above the head or below the feet
            }
            let hit = if west {
                // x-plane at ecx·3, spanning z from ecz·3.
                let px = ecx as f32 * BUILD_CELL_M;
                let s0 = ecz as f32 * BUILD_CELL_M;
                edge_hit(px, s0, x, nx, nz, posts_only)
            } else {
                // z-plane at ecz·3, spanning x from ecx·3.
                let pz = ecz as f32 * BUILD_CELL_M;
                let s0 = ecx as f32 * BUILD_CELL_M;
                edge_hit(pz, s0, z, nz, nx, posts_only)
            };
            if hit {
                return true;
            }
        }
    }
    false
}

/// Whether a wall or doorway post stops the horizontal move
/// (x,z)→(nx,nz) at `feet_y`. Movement steps are ≤ 0.19 m, so testing
/// the endpoint's z against the edge span (instead of the exact crossing
/// point) cuts at most a fingertip off a post corner.
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
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build::{
        place, BuildContent, PieceDef, Pieces, LOC_PLANE, LOC_RISER, MAT_WOOD, SHAPE_DOORWAY,
        SHAPE_STAIRS,
    };
    use crate::deploy::Deploys;
    use crate::input::InputFrame;
    use crate::movement::{self, Body, POS_XZ_Q, POS_Y_Q};
    use crate::rng::Pcg32;
    use crate::world::{EventQueue, Player, EV_PIECE_PLACED};

    /// The browser-smoke seed and its guarded-walkable cell (build.rs
    /// tests use the same anchors).
    const SEED: u64 = 20260731;
    const CX: u16 = 341;
    const CZ: u16 = 341;

    /// Free pieces (n_costs 0) so tests place without inventories: rows
    /// foundation, wall, doorway, floor, stairs.
    fn free_table() -> BuildContent {
        let shapes = [
            SHAPE_FOUNDATION,
            SHAPE_WALL,
            SHAPE_DOORWAY,
            SHAPE_FLOOR,
            SHAPE_STAIRS,
        ];
        let mut b = BuildContent::EMPTY;
        b.piece_count = shapes.len() as u16;
        for (i, &shape) in shapes.iter().enumerate() {
            b.pieces[i] = PieceDef {
                shape,
                material: MAT_WOOD,
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
        let wall_x = CX as f32 * BUILD_CELL_M; // 1023: the west edge plane

        // A wall on the west edge stops a westward walk at the slab.
        let mut pieces = Pieces::new();
        put(&bc, &mut pieces, CX, CZ, 0, LOC_PLANE, 0);
        put(&bc, &mut pieces, CX, CZ, 0, crate::build::LOC_EDGE_W, 1);
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
        put(&bc, &mut pieces, CX, CZ, 0, crate::build::LOC_EDGE_W, 2);
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

        // Aimed at a post (z inside the south post span): blocked.
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
        // foundation + north wall carry a level-1 floor, which supports a
        // level-1 west wall over a bare level-0 west edge — walking out
        // west at ground level passes under it.
        let mut pieces = Pieces::new();
        put(&bc, &mut pieces, CX, CZ, 0, LOC_PLANE, 0);
        put(&bc, &mut pieces, CX, CZ, 0, crate::build::LOC_EDGE_N, 1);
        put(&bc, &mut pieces, CX, CZ, 1, LOC_PLANE, 3);
        put(&bc, &mut pieces, CX, CZ, 1, crate::build::LOC_EDGE_W, 1);
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
        put(&bc, &mut pieces, CX, CZ, 0, crate::build::LOC_EDGE_W, 1);
        put(&bc, &mut pieces, CX, CZ, 1, crate::build::LOC_EDGE_W, 1);
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

        // Walking east off the slab falls back to terrain.
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
        ];
        for _ in 0..20_000 {
            let cx = rng.next_bounded(24) as u16;
            let cz = rng.next_bounded(24) as u16;
            let level = rng.next_bounded(8) as u8;
            let shape = shapes[rng.next_bounded(5) as usize];
            let loc = match shape {
                SHAPE_WALL | SHAPE_DOORWAY => (2 + rng.next_bounded(2)) as u8,
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
        put(&bc, &mut pieces, CX, CZ, 0, crate::build::LOC_EDGE_W, 1);
        assert_eq!(pieces.cols().get(CX, CZ).walls_w, 1);
        // The sweep's removal path: drop the wall by store index.
        let wi = pieces
            .entries()
            .iter()
            .position(|r| r.loc == crate::build::LOC_EDGE_W)
            .unwrap();
        pieces.remove_at(wi, SHAPE_WALL);
        assert_eq!(pieces.cols().get(CX, CZ).walls_w, 0);
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
