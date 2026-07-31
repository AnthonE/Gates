//! Build — the third survival verb (DESIGN.md §2, M1): foundation-grid
//! building. A place request names a baked piece row from
//! `content/building.toml` and a grid address; the sim validates support,
//! terrain, reach, and cost, pays the cost from the placer's inventory,
//! and records the piece. Pure and fixed-capacity like gather and craft:
//! content reaches it only as the baked `BuildContent` table, the inert
//! `EMPTY` default makes build a no-op, and `probe_fixture()` is the
//! synthetic table for the parity/replay/alloc gates.
//!
//! Grid model v0 (proposed defaults, DECISIONS.md §open "build grid v0"):
//! world-axis-aligned 3 m cells × 3 m levels. Each (cell, level) holds one
//! **plane** piece (foundation at level 0, floor/roof above) and one
//! **riser** (stairs); each cell boundary holds one **edge** piece (wall/
//! doorway) per level, canonicalized to the cell's west/north side. Pieces
//! are movement collision (collide.rs): the store keeps a derived column
//! index in lockstep so the tick's collision queries are O(1), and the
//! client's mirror keeps its own copy for the predictor.
//!
//! Support rules v0: a foundation needs buildable terrain; an edge piece
//! needs a foundation beside it (level 0) or an edge piece below / a plane
//! beside it (higher levels); a plane above ground needs an edge piece
//! under one of its four sides; stairs need the plane they stand on.
//! Hearth privilege gates placement (deploy.rs: a foreign hearth's radius
//! refuses with `REFUSE_B_CLAIM`), and pieces carry hp + an upkeep clock
//! for the decay sweep (deploy.rs `upkeep_sweep`). Upgrade-in-place and
//! piece collision are the next slice (NOW.md).

use crate::craft::{inv_count, inv_take};
use crate::deploy::{Deploys, UPKEEP_PERIOD_TICKS};
use crate::fmath::floor_i32;
use crate::limits::{
    MAX_BUILD_COORD, MAX_BUILD_LEVELS, MAX_PIECES, MAX_PIECE_COSTS, MAX_PIECE_DEFS,
};
use crate::terrain;
use crate::world::{EventQueue, Player, EV_BUILD_REFUSED, EV_PIECE_PLACED};

/// Shape codes (schema order: CONTENT.md §1 building_piece).
pub const SHAPE_FOUNDATION: u8 = 0;
pub const SHAPE_WALL: u8 = 1;
pub const SHAPE_DOORWAY: u8 = 2;
pub const SHAPE_FLOOR: u8 = 3;
pub const SHAPE_STAIRS: u8 = 4;
pub const SHAPE_ROOF: u8 = 5;

/// Material codes (schema order: wood → stone → metal).
pub const MAT_WOOD: u8 = 0;
pub const MAT_STONE: u8 = 1;
pub const MAT_METAL: u8 = 2;

/// Grid locations within a cell. Planes and risers occupy the cell body;
/// edge pieces are canonical to the cell's west (x = cx·3 m) or north
/// (z = cz·3 m) boundary — the same physical edge is never addressable
/// twice.
pub const LOC_PLANE: u8 = 0;
pub const LOC_RISER: u8 = 1;
pub const LOC_EDGE_W: u8 = 2;
pub const LOC_EDGE_N: u8 = 3;

/// Integer refusal reasons (CLAUDE.md wall 3), carried by
/// EV_BUILD_REFUSED / the build-refused wire subtype.
pub const REFUSE_B_PIECE: u32 = 0;
pub const REFUSE_B_SPOT: u32 = 1;
pub const REFUSE_B_SUPPORT: u32 = 2;
pub const REFUSE_B_TERRAIN: u32 = 3;
pub const REFUSE_B_REACH: u32 = 4;
pub const REFUSE_B_COST: u32 = 5;
pub const REFUSE_B_FULL: u32 = 6;
pub const REFUSE_B_CLAIM: u32 = 7;

/// Build cell size in meters (v0: one foundation spans one cell).
/// Proposed default, DECISIONS.md §open ("build grid v0").
pub const BUILD_CELL_M: f32 = 3.0;
/// Storey height in meters. Proposed default, same §open row.
pub const LEVEL_H_M: f32 = 3.0;
/// Placement reach in meters, planar, player to piece anchor. Proposed
/// default, same §open row.
pub const BUILD_REACH_M: f32 = 5.0;
/// Foundation terrain rules: cell-center height at least this (above the
/// beach line, so the sea floor can't hold a base) and slope under this
/// (the spawn walkability shape). Proposed defaults, same §open row.
pub const FOUNDATION_MIN_H_M: f32 = 1.5;
pub const FOUNDATION_MAX_SLOPE: f32 = 1.0;

/// One baked piece row. `hp == 0` ⇒ inert (the empty-table row).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PieceDef {
    pub shape: u8,
    pub material: u8,
    pub hp: u16,
    /// Live rows in `costs`.
    pub n_costs: u8,
    /// (item index, units) — paid whole at placement.
    pub costs: [(u16, u16); MAX_PIECE_COSTS],
}

impl PieceDef {
    pub const INERT: Self = Self {
        shape: SHAPE_FOUNDATION,
        material: MAT_WOOD,
        hp: 0,
        n_costs: 0,
        costs: [(0, 0); MAX_PIECE_COSTS],
    };
}

/// The whole build ruleset the sim knows. Construction input like the
/// gather table: the boot path bakes it from `content/building.toml`
/// before the first tick, and the WAL pins the content hash it came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BuildContent {
    pub pieces: [PieceDef; MAX_PIECE_DEFS],
    pub piece_count: u16,
}

impl BuildContent {
    /// Inert: no piece exists, every request refuses. `World::new` starts
    /// here.
    pub const EMPTY: Self = Self {
        pieces: [PieceDef::INERT; MAX_PIECE_DEFS],
        piece_count: 0,
    };

    /// Synthetic table for the parity/replay/alloc gates, over the gather
    /// probe fixture's items (fixture, not game content). Row 2 costs a
    /// different item so multi-material inventories are inside the gates;
    /// row 3 is the doorway a door deployable needs (the door slice).
    pub fn probe_fixture() -> Self {
        let mut b = Self::EMPTY;
        b.piece_count = 4;
        b.pieces[0] = PieceDef {
            shape: SHAPE_FOUNDATION,
            material: MAT_WOOD,
            hp: 100,
            n_costs: 1,
            costs: [(0, 5), (0, 0)],
        };
        b.pieces[1] = PieceDef {
            shape: SHAPE_WALL,
            material: MAT_WOOD,
            hp: 100,
            n_costs: 1,
            costs: [(0, 3), (0, 0)],
        };
        b.pieces[2] = PieceDef {
            shape: SHAPE_FLOOR,
            material: MAT_STONE,
            hp: 150,
            n_costs: 1,
            costs: [(1, 3), (0, 0)],
        };
        b.pieces[3] = PieceDef {
            shape: SHAPE_DOORWAY,
            material: MAT_WOOD,
            hp: 100,
            n_costs: 1,
            costs: [(0, 3), (0, 0)],
        };
        b
    }
}

/// One placed piece. Its grid address (cx, cz, level, loc) is its
/// identity — pieces don't move and the wire refers to them by address.
/// `hp` and `uh` are sim-only (the wire carries address + row; clients
/// learn of decay by the removal broadcast, not by watching hp).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PieceRec {
    pub cx: u16,
    pub cz: u16,
    pub level: u8,
    pub loc: u8,
    /// Baked piece row this address holds.
    pub row: u8,
    /// Current hp (decay drains it; piece damage lands in M2).
    pub hp: u16,
    /// Last upkeep period processed (`tick / UPKEEP_PERIOD_TICKS`).
    pub uh: u16,
}

/// The placed-piece store: dense, insertion-ordered (command order, so
/// iteration is deterministic). Decay removal swap-removes (the wire
/// layer restarts in-progress sync walks on any removal). Overflow
/// refuses the placement (limits.rs `MAX_PIECES`). The column index is
/// derived collision state (collide.rs) — maintained in lockstep here,
/// never hashed, exactly like the event ring. Boxed: it is built once at
/// construction (boot path — wall 2 counts the tick, and the tick only
/// reads and flips bits in place) and keeping its 160 KB off the stack
/// is what lets tests and the wasm probe build Worlds on default stacks.
pub struct Pieces {
    entries: [PieceRec; MAX_PIECES],
    len: usize,
    cols: Box<crate::collide::ColIndex>,
}

impl Pieces {
    pub fn new() -> Self {
        Self {
            entries: [PieceRec::default(); MAX_PIECES],
            len: 0,
            cols: Box::new(crate::collide::ColIndex::new()),
        }
    }

    /// The collision view movement steps against (collide.rs).
    pub fn cols(&self) -> &crate::collide::ColIndex {
        &self.cols
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn entries(&self) -> &[PieceRec] {
        &self.entries[..self.len]
    }

    pub fn find(&self, cx: u16, cz: u16, level: u8, loc: u8) -> Option<&PieceRec> {
        self.entries[..self.len]
            .iter()
            .find(|p| p.cx == cx && p.cz == cz && p.level == level && p.loc == loc)
    }

    /// Append a record. False ⇒ store full (the caller refuses the
    /// placement; nothing is evicted). `shape` keeps the column index in
    /// lockstep — the caller has the baked row in hand.
    fn insert(&mut self, rec: PieceRec, shape: u8) -> bool {
        if self.len == MAX_PIECES {
            return false;
        }
        self.entries[self.len] = rec;
        self.len += 1;
        self.cols.add(rec.cx, rec.cz, rec.level, rec.loc, shape);
        true
    }

    /// Swap-remove entry `i` (the decay sweep's removal; deploy.rs).
    pub(crate) fn remove_at(&mut self, i: usize, shape: u8) {
        let rec = self.entries[i];
        self.cols.del(rec.cx, rec.cz, rec.level, rec.loc, shape);
        self.len -= 1;
        self.entries[i] = self.entries[self.len];
    }

    /// Update entry `i`'s upkeep state (the decay sweep's write-back).
    pub(crate) fn set_upkeep(&mut self, i: usize, hp: u16, uh: u16) {
        self.entries[i].hp = hp;
        self.entries[i].uh = uh;
    }

    /// Set or clear the closed-door bit at a doorway edge (deploy.rs owns
    /// when: door placement, the use toggle, and door removal).
    pub(crate) fn set_door(&mut self, cx: u16, cz: u16, level: u8, loc: u8, shut: bool) {
        self.cols.set_door(cx, cz, level, loc, shut);
    }
}

impl Default for Pieces {
    fn default() -> Self {
        Self::new()
    }
}

/// Whether (cx, cz, level) holds a plane piece (foundation/floor/roof).
fn plane_at(pieces: &Pieces, cx: u16, cz: u16, level: u8) -> bool {
    pieces.find(cx, cz, level, LOC_PLANE).is_some()
}

/// Whether the edge address holds a wall or doorway.
fn edge_at(pieces: &Pieces, cx: u16, cz: u16, level: u8, loc: u8) -> bool {
    pieces.find(cx, cz, level, loc).is_some()
}

/// The two cells an edge piece adjoins: the canonical cell and its west or
/// north neighbor (None past the grid's low border).
fn edge_neighbors(cx: u16, cz: u16, loc: u8) -> ((u16, u16), Option<(u16, u16)>) {
    if loc == LOC_EDGE_W {
        ((cx, cz), cx.checked_sub(1).map(|x| (x, cz)))
    } else {
        ((cx, cz), cz.checked_sub(1).map(|z| (cx, z)))
    }
}

/// The planar anchor point of an address — what reach is measured to.
fn anchor(cx: u16, cz: u16, loc: u8) -> (f32, f32) {
    let x0 = cx as f32 * BUILD_CELL_M;
    let z0 = cz as f32 * BUILD_CELL_M;
    let half = BUILD_CELL_M * 0.5;
    match loc {
        LOC_EDGE_W => (x0, z0 + half),
        LOC_EDGE_N => (x0 + half, z0),
        _ => (x0 + half, z0 + half),
    }
}

/// Build cell of a world x/z coordinate.
#[inline]
pub fn build_cell_of(v: f32) -> i32 {
    floor_i32(v / BUILD_CELL_M)
}

/// Whether `loc` is the kind of slot `shape` occupies.
fn loc_fits_shape(shape: u8, loc: u8) -> bool {
    match shape {
        SHAPE_FOUNDATION | SHAPE_FLOOR | SHAPE_ROOF => loc == LOC_PLANE,
        SHAPE_STAIRS => loc == LOC_RISER,
        SHAPE_WALL | SHAPE_DOORWAY => loc == LOC_EDGE_W || loc == LOC_EDGE_N,
        _ => false,
    }
}

/// Support rule v0 for a piece of `shape` at the address. Foundations
/// carry no support requirement (terrain is their check).
fn supported(pieces: &Pieces, shape: u8, cx: u16, cz: u16, level: u8, loc: u8) -> bool {
    match shape {
        SHAPE_FOUNDATION => true,
        SHAPE_FLOOR | SHAPE_ROOF => {
            // An edge piece under any of the cell's four sides.
            level >= 1
                && (edge_at(pieces, cx, cz, level - 1, LOC_EDGE_W)
                    || edge_at(pieces, cx + 1, cz, level - 1, LOC_EDGE_W)
                    || edge_at(pieces, cx, cz, level - 1, LOC_EDGE_N)
                    || edge_at(pieces, cx, cz + 1, level - 1, LOC_EDGE_N))
        }
        SHAPE_STAIRS => plane_at(pieces, cx, cz, level),
        SHAPE_WALL | SHAPE_DOORWAY => {
            let ((ax, az), other) = edge_neighbors(cx, cz, loc);
            if level == 0 {
                plane_at(pieces, ax, az, 0)
                    || other.is_some_and(|(bx, bz)| plane_at(pieces, bx, bz, 0))
            } else {
                edge_at(pieces, cx, cz, level - 1, loc)
                    || plane_at(pieces, ax, az, level)
                    || other.is_some_and(|(bx, bz)| plane_at(pieces, bx, bz, level))
            }
        }
        _ => false,
    }
}

/// Apply one place request (`Command::Place`). Refusals are events, not
/// errors — the placer hears why. The cost is paid whole at placement;
/// the piece is announced by EV_PIECE_PLACED for the wire to broadcast.
/// `deploys` carries the hearth list for the privilege check; `tick`
/// stamps the piece's upkeep clock (deploy.rs).
#[allow(clippy::too_many_arguments)]
pub fn place(
    seed: u64,
    bc: &BuildContent,
    deploys: &Deploys,
    pieces: &mut Pieces,
    p: &mut Player,
    tick: u64,
    row: u16,
    cx: u16,
    cz: u16,
    level: u8,
    loc: u8,
    events: &mut EventQueue,
) {
    if row >= bc.piece_count {
        events.push(EV_BUILD_REFUSED, p.id, REFUSE_B_PIECE, 0);
        return;
    }
    let def = &bc.pieces[row as usize];
    if def.hp == 0 {
        events.push(EV_BUILD_REFUSED, p.id, REFUSE_B_PIECE, 0);
        return;
    }
    let level_ok = (level as usize) < MAX_BUILD_LEVELS
        && (def.shape != SHAPE_FOUNDATION || level == 0)
        && (!matches!(def.shape, SHAPE_FLOOR | SHAPE_ROOF) || level >= 1);
    if (cx as usize) >= MAX_BUILD_COORD
        || (cz as usize) >= MAX_BUILD_COORD
        || !level_ok
        || !loc_fits_shape(def.shape, loc)
        || pieces.find(cx, cz, level, loc).is_some()
    {
        events.push(EV_BUILD_REFUSED, p.id, REFUSE_B_SPOT, 0);
        return;
    }
    let (ax, az) = anchor(cx, cz, loc);
    let px = p.body.qx as f32 * crate::movement::POS_XZ_Q;
    let pz = p.body.qz as f32 * crate::movement::POS_XZ_Q;
    let (dx, dz) = (ax - px, az - pz);
    if dx * dx + dz * dz > BUILD_REACH_M * BUILD_REACH_M {
        events.push(EV_BUILD_REFUSED, p.id, REFUSE_B_REACH, 0);
        return;
    }
    if deploys.foreign_claim(ax, az, p.id) {
        events.push(EV_BUILD_REFUSED, p.id, REFUSE_B_CLAIM, 0);
        return;
    }
    if def.shape == SHAPE_FOUNDATION {
        let h = terrain::height(seed, ax, az);
        if h < FOUNDATION_MIN_H_M || terrain::slope(seed, ax, az) >= FOUNDATION_MAX_SLOPE {
            events.push(EV_BUILD_REFUSED, p.id, REFUSE_B_TERRAIN, 0);
            return;
        }
    }
    if !supported(pieces, def.shape, cx, cz, level, loc) {
        events.push(EV_BUILD_REFUSED, p.id, REFUSE_B_SUPPORT, 0);
        return;
    }
    for &(item, units) in def.costs.iter().take(def.n_costs as usize) {
        if inv_count(&p.inv, item) < units as u32 {
            events.push(EV_BUILD_REFUSED, p.id, REFUSE_B_COST, 0);
            return;
        }
    }
    let rec = PieceRec {
        cx,
        cz,
        level,
        loc,
        row: row as u8,
        hp: def.hp,
        uh: (tick / UPKEEP_PERIOD_TICKS) as u16,
    };
    if !pieces.insert(rec, def.shape) {
        events.push(EV_BUILD_REFUSED, p.id, REFUSE_B_FULL, 0);
        return;
    }
    for &(item, units) in def.costs.iter().take(def.n_costs as usize) {
        inv_take(&mut p.inv, item, units as u32);
    }
    events.push(
        EV_PIECE_PLACED,
        crate::gather::cell_key(cx, cz),
        ((level as u32) << 16) | ((loc as u32) << 8) | row as u32,
        0,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gather::ItemStack;
    use crate::movement::Body;
    use crate::world::{EventQueue, Player};

    /// A seed whose island center is buildable (the browser-smoke seed;
    /// `world::tests` guards its walkability natively).
    const SEED: u64 = 20260731;
    /// The smoke spawn's build cell: (1024 m, 1024 m) / 3 m.
    const CX: u16 = 341;
    const CZ: u16 = 341;

    fn player_at_cell_center(items: &[(u16, u16)]) -> Player {
        let mut p = Player {
            id: 7,
            active: true,
            body: Body::at(
                SEED,
                (CX as f32 + 0.5) * BUILD_CELL_M,
                (CZ as f32 + 0.5) * BUILD_CELL_M,
            ),
            ..Player::default()
        };
        for (i, &(item, count)) in items.iter().enumerate() {
            p.inv[i] = ItemStack { item, count };
        }
        p
    }

    fn last(ev: &EventQueue) -> (u8, u32, u32) {
        let e = ev.entries()[ev.len() - 1];
        (e.code, e.a, e.b)
    }

    #[test]
    fn foundation_wall_floor_chain_places_and_pays() {
        let bc = BuildContent::probe_fixture();
        let mut pieces = Pieces::new();
        let nod = Deploys::new();
        let mut ev = EventQueue::default();
        let mut p = player_at_cell_center(&[(0, 20), (1, 10)]);

        place(
            SEED,
            &bc,
            &nod,
            &mut pieces,
            &mut p,
            0,
            0,
            CX,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(
            last(&ev),
            (
                crate::world::EV_PIECE_PLACED,
                crate::gather::cell_key(CX, CZ),
                0
            )
        );
        assert_eq!(inv_count(&p.inv, 0), 15, "foundation cost paid");

        place(
            SEED,
            &bc,
            &nod,
            &mut pieces,
            &mut p,
            0,
            1,
            CX,
            CZ,
            0,
            LOC_EDGE_W,
            &mut ev,
        );
        assert_eq!(last(&ev).0, crate::world::EV_PIECE_PLACED);
        assert_eq!(inv_count(&p.inv, 0), 12, "wall cost paid");

        // Floor at level 1 stands on the wall below.
        place(
            SEED,
            &bc,
            &nod,
            &mut pieces,
            &mut p,
            0,
            2,
            CX,
            CZ,
            1,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(last(&ev).0, crate::world::EV_PIECE_PLACED);
        assert_eq!(inv_count(&p.inv, 1), 7);
        assert_eq!(pieces.len(), 3);

        // Wall at level 1 stacks on the wall at level 0.
        place(
            SEED,
            &bc,
            &nod,
            &mut pieces,
            &mut p,
            0,
            1,
            CX,
            CZ,
            1,
            LOC_EDGE_W,
            &mut ev,
        );
        assert_eq!(last(&ev).0, crate::world::EV_PIECE_PLACED);
    }

    #[test]
    fn refusals_name_their_reason_and_change_nothing() {
        let bc = BuildContent::probe_fixture();
        let mut pieces = Pieces::new();
        let nod = Deploys::new();
        let mut ev = EventQueue::default();
        let mut p = player_at_cell_center(&[(0, 100), (1, 100)]);

        // Bad row; wrong loc for the shape; foundation above ground.
        let cases: [(u16, u8, u8, u32); 4] = [
            (9, 0, LOC_PLANE, REFUSE_B_PIECE),
            (0, 0, LOC_EDGE_W, REFUSE_B_SPOT),
            (0, 1, LOC_PLANE, REFUSE_B_SPOT),
            (2, 0, LOC_PLANE, REFUSE_B_SPOT), // floor at level 0
        ];
        for (row, level, loc, reason) in cases {
            place(
                SEED,
                &bc,
                &nod,
                &mut pieces,
                &mut p,
                0,
                row,
                CX,
                CZ,
                level,
                loc,
                &mut ev,
            );
            assert_eq!(last(&ev), (crate::world::EV_BUILD_REFUSED, 7, reason));
        }

        // Reach: a cell 20 m away.
        place(
            SEED,
            &bc,
            &nod,
            &mut pieces,
            &mut p,
            0,
            0,
            CX + 7,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(last(&ev).1, 7);
        assert_eq!(last(&ev).2, REFUSE_B_REACH);

        // Support: a wall with no foundation beside it.
        place(
            SEED,
            &bc,
            &nod,
            &mut pieces,
            &mut p,
            0,
            1,
            CX,
            CZ,
            0,
            LOC_EDGE_N,
            &mut ev,
        );
        assert_eq!(last(&ev).2, REFUSE_B_SUPPORT);
        // Floor with no wall below it.
        place(
            SEED,
            &bc,
            &nod,
            &mut pieces,
            &mut p,
            0,
            2,
            CX,
            CZ,
            1,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(last(&ev).2, REFUSE_B_SUPPORT);

        // Cost: place the foundation with an empty inventory.
        let mut poor = player_at_cell_center(&[]);
        place(
            SEED,
            &bc,
            &nod,
            &mut pieces,
            &mut poor,
            0,
            0,
            CX,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(last(&ev).2, REFUSE_B_COST);

        // Occupied: place the foundation twice.
        place(
            SEED,
            &bc,
            &nod,
            &mut pieces,
            &mut p,
            0,
            0,
            CX,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(last(&ev).0, crate::world::EV_PIECE_PLACED);
        place(
            SEED,
            &bc,
            &nod,
            &mut pieces,
            &mut p,
            0,
            0,
            CX,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(last(&ev).2, REFUSE_B_SPOT);
        assert_eq!(pieces.len(), 1, "refusals inserted nothing");
    }

    #[test]
    fn terrain_refuses_the_sea() {
        let bc = BuildContent::probe_fixture();
        let mut pieces = Pieces::new();
        let nod = Deploys::new();
        let mut ev = EventQueue::default();
        // Cell (1,1) is deep sea on every island seed (coast radius starts
        // ~800 m in); stand the player there to isolate the terrain rule.
        let mut p = player_at_cell_center(&[(0, 100)]);
        p.body = Body::at(SEED, 1.5 * BUILD_CELL_M, 1.5 * BUILD_CELL_M);
        place(
            SEED,
            &bc,
            &nod,
            &mut pieces,
            &mut p,
            0,
            0,
            1,
            1,
            0,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(last(&ev).2, REFUSE_B_TERRAIN);
    }

    #[test]
    fn store_full_refuses_not_evicts() {
        let mut pieces = Pieces::new();
        let nod = Deploys::new();
        for i in 0..MAX_PIECES {
            assert!(pieces.insert(
                PieceRec {
                    cx: (i % MAX_BUILD_COORD) as u16,
                    cz: (i / MAX_BUILD_COORD) as u16,
                    level: 0,
                    loc: LOC_PLANE,
                    row: 0,
                    hp: 1,
                    uh: 0,
                },
                SHAPE_FOUNDATION
            ));
        }
        assert!(!pieces.insert(PieceRec::default(), SHAPE_FOUNDATION));
        assert_eq!(pieces.len(), MAX_PIECES);

        let bc = BuildContent::probe_fixture();
        let mut ev = EventQueue::default();
        let mut p = player_at_cell_center(&[(0, 100)]);
        place(
            SEED,
            &bc,
            &nod,
            &mut pieces,
            &mut p,
            0,
            0,
            CX,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(last(&ev).2, REFUSE_B_FULL);
        assert_eq!(inv_count(&p.inv, 0), 100, "nothing paid on a full store");
    }

    #[test]
    fn edge_canonicalization_shares_the_boundary() {
        let bc = BuildContent::probe_fixture();
        let mut pieces = Pieces::new();
        let nod = Deploys::new();
        let mut ev = EventQueue::default();
        let mut p = player_at_cell_center(&[(0, 100)]);
        // Foundation in the cell EAST of the wall's canonical cell: the
        // wall at (CX+1, W) adjoins cells CX and CX+1 — support must come
        // from either side.
        place(
            SEED,
            &bc,
            &nod,
            &mut pieces,
            &mut p,
            0,
            0,
            CX,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
        );
        place(
            SEED,
            &bc,
            &nod,
            &mut pieces,
            &mut p,
            0,
            1,
            CX + 1,
            CZ,
            0,
            LOC_EDGE_W,
            &mut ev,
        );
        assert_eq!(last(&ev).0, crate::world::EV_PIECE_PLACED);
    }
}
