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
//! for the decay sweep (deploy.rs `upkeep_sweep`).
//!
//! Upgrade-in-place (`upgrade`) moves a standing piece up the material
//! ladder without tearing it down: same shape, same address, same
//! collision, a higher material's hp and upkeep. Piece damage is M2.
//!
//! **Placement and grade are two acts** (twig v0, 2026-08-10,
//! `reference/BUILDING.md` §7b.4). A piece enters the world as **twig**
//! and only as twig — `place` refuses every other rung — so the shape of
//! a base is drafted at a tenth of its price and `upgrade` is what
//! commits it, paying the grade on top of the twig already spent. Twig
//! is 10 hp and is the one material `upkeep_sweep` never protects: a
//! scaffold nobody pays rent on rots on its own clock (`deploy.rs`).
//! That is what makes it a draft rather than a cheap permanent base.

use crate::craft::{inv_count, inv_take};
use crate::deploy::{DeployContent, Deploys, UPKEEP_PERIOD_TICKS};
use crate::fmath::floor_i32;
use crate::limits::{
    MAX_BUILD_COORD, MAX_BUILD_LEVELS, MAX_COLLAPSE_PIECES, MAX_DEPLOY_COSTS, MAX_PIECES,
    MAX_PIECE_COSTS, MAX_PIECE_DEFS, SUPPORT_SWEEP_PER_TICK,
};
use crate::terrain;
use crate::world::{
    EventQueue, Player, EV_BUILD_REFUSED, EV_PIECE_PLACED, EV_PIECE_REPAIRED, STRUCT_DEPLOY_BIT,
};

/// Shape codes (schema order: CONTENT.md §1 building_piece).
pub const SHAPE_FOUNDATION: u8 = 0;
pub const SHAPE_WALL: u8 = 1;
pub const SHAPE_DOORWAY: u8 = 2;
pub const SHAPE_FLOOR: u8 = 3;
pub const SHAPE_STAIRS: u8 = 4;
pub const SHAPE_ROOF: u8 = 5;
/// The two socket shapes (`reference/BUILDING.md` §9.13, catalogue v1):
/// openings that will one day hold an insert the way a doorway holds a
/// door. Until the inserts exist they are holes with rules — the window
/// **blocks a body and not an arrow** (its aperture is `collide.rs`'s
/// `WINDOW_SILL_M..WINDOW_HEAD_M` band), the frame blocks neither. Edge
/// pieces like the wall, and they fill `SHAPE_BITS`' last two codes, which
/// is why they cost no wire widening.
pub const SHAPE_WINDOW: u8 = 6;
pub const SHAPE_FRAME: u8 = 7;

/// Material codes (schema order: twig → wood → stone → metal). The order
/// is the ladder: `upgrade` climbs it by comparing these numbers, so a
/// rung inserted below wood has to renumber everything above it — which
/// is what twig did (wire v34, `protocol/src/lib.rs`).
///
/// **`MAT_TWIG` is not a grade, it is the placement state.** Every piece
/// enters the world as twig and nothing else (`place` refuses any other
/// row); a hammer commits it upward. `reference/BUILDING.md` §7b.4 is the
/// model and DECISIONS.md §open "twig v0" is the row.
pub const MAT_TWIG: u8 = 0;
pub const MAT_WOOD: u8 = 1;
pub const MAT_STONE: u8 = 2;
pub const MAT_METAL: u8 = 3;

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
///
/// **Growing this list is still a two-file act, but NOTHING ENFORCES IT
/// ANY MORE.** This comment used to say `ci/ui_smoke.mjs` §W walked these
/// constants by name and by value against `BUILD_REFUSE_TEXT` in
/// `web/src/interact.js`, so a code with no sentence turned a gate red.
/// That gate went with the browser client (operator, 2026-08-06) and has
/// no native replacement — `ci/` holds no `.mjs` of that name. The
/// *hazard* is unchanged and is now unwatched: a code with no sentence
/// reaches the player as `can't build: code N`, and the only thing
/// standing between here and that is remembering to add the sentence in
/// `crates/client/src/ui/` in the same commit. It went wrong twice while
/// the gate existed (`REFUSE_B_INTACT`, `REFUSE_B_UNPRICED`), which is
/// the measure of how easy it is to forget without one.
pub const REFUSE_B_PIECE: u32 = 0;
pub const REFUSE_B_SPOT: u32 = 1;
pub const REFUSE_B_SUPPORT: u32 = 2;
pub const REFUSE_B_TERRAIN: u32 = 3;
pub const REFUSE_B_REACH: u32 = 4;
pub const REFUSE_B_COST: u32 = 5;
pub const REFUSE_B_FULL: u32 = 6;
pub const REFUSE_B_CLAIM: u32 = 7;
/// **The ladder said no**, for either of the two verbs that climb it. On
/// `upgrade`: the named material is not a rung above this piece's, or the
/// table holds no such rung for its shape. On `place`: the named row is
/// not twig, and twig is the only thing a placement may be (twig v0).
pub const REFUSE_B_TIER: u32 = 8;
/// The repair verb's own refusal: the piece is already at its baked hp, so
/// there is nothing to buy. Also the answer on an unbaked table
/// (`repair_pct == 0`), where healing free is the alternative.
pub const REFUSE_B_INTACT: u32 = 9;
/// The repair verb's second refusal: the target's baked row quotes no
/// price at all (`n_costs == 0`), so there is nothing to charge. A cost
/// loop over zero rows takes zero materials and mends anyway, which is the
/// free heal the whole price exists to refuse — this is that hole named
/// and closed for both stores. A deployable reaches it when content
/// carries no recipe for its item.
pub const REFUSE_B_UNPRICED: u32 = 10;
/// The grace window is spent (`limits.rs` `DEMOLISH_WINDOW_TICKS`). A
/// piece older than its window comes down by explosives and nothing else
/// — demolish is a mistake-fix, not a verb
/// (`reference/BUILDING.md` §6).
pub const REFUSE_B_WINDOW: u32 = 11;
/// The address holds nothing to take down.
pub const REFUSE_B_EMPTY: u32 = 12;

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
        material: MAT_TWIG,
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
    /// Repair price, percent of the pro-rata share of a piece's own build
    /// cost (`content/balance.toml` `repair_cost_pct`; 100 = the damage's
    /// worth exactly). Content validates 1..=100, so a zero here means the
    /// table was never baked — and `repair` refuses on a zero rather than
    /// healing free.
    pub repair_pct: u16,
}

impl BuildContent {
    /// Inert: no piece exists, every request refuses. `World::new` starts
    /// here.
    pub const EMPTY: Self = Self {
        pieces: [PieceDef::INERT; MAX_PIECE_DEFS],
        piece_count: 0,
        repair_pct: 0,
    };

    /// Synthetic table for the parity/replay/alloc gates, over the gather
    /// probe fixture's items (fixture, not game content). Row 2 costs a
    /// different item so multi-material inventories are inside the gates;
    /// row 3 is the doorway a door deployable needs (the door slice);
    /// row 4 is row 1's stone rung, so the upgrade verb has somewhere to
    /// climb to (the upgrade slice) — and it costs the second item, so an
    /// upgrade's payment is distinguishable from the wall's own.
    ///
    /// **Rows 0-3 are twig** since twig v0, because those are the rows
    /// everything *places* and a placement may not be anything else. Row
    /// 2 keeps the second cost item that is its whole reason for
    /// existing; only its rung moved. Rows 4, 5 and 6 are the stone rungs
    /// of rows 1, 0 and 2, so they are both the upgrade verb's targets
    /// and — being unplaceable — what drives `REFUSE_B_TIER` through the
    /// probes, which cycle rows `0..6`. Every hp and every cost of rows
    /// 0-4 is unchanged from when rows 0-3 were labelled wood: nothing
    /// about the probes' arithmetic moved, only which rung the label
    /// names. The **doorway is the one shape with nothing above it**,
    /// which is what a missing-rung refusal is tested against.
    pub fn probe_fixture() -> Self {
        let mut b = Self::EMPTY;
        b.piece_count = 7;
        // The shipped default, so a repair driven through this fixture is
        // priced the way a shard prices one. Setting it makes the verb
        // *possible* here and nothing more — what puts it inside the
        // parity, replay and alloc gates is those gates issuing
        // `Command::Repair`, which `probe.rs`, `tests/replay.rs` and
        // `tests/alloc_zero.rs` each do. An earlier version of this
        // comment claimed the price alone did it; it did not.
        b.repair_pct = 100;
        b.pieces[0] = PieceDef {
            shape: SHAPE_FOUNDATION,
            material: MAT_TWIG,
            hp: 100,
            n_costs: 1,
            costs: [(0, 5), (0, 0)],
        };
        b.pieces[1] = PieceDef {
            shape: SHAPE_WALL,
            material: MAT_TWIG,
            hp: 100,
            n_costs: 1,
            costs: [(0, 3), (0, 0)],
        };
        b.pieces[2] = PieceDef {
            shape: SHAPE_FLOOR,
            material: MAT_TWIG,
            hp: 150,
            n_costs: 1,
            costs: [(1, 3), (0, 0)],
        };
        b.pieces[3] = PieceDef {
            shape: SHAPE_DOORWAY,
            material: MAT_TWIG,
            hp: 100,
            n_costs: 1,
            costs: [(0, 3), (0, 0)],
        };
        b.pieces[4] = PieceDef {
            shape: SHAPE_WALL,
            material: MAT_STONE,
            hp: 200,
            n_costs: 1,
            costs: [(1, 4), (0, 0)],
        };
        // Row 5 is row 0's stone rung — the foundation ladder, added with
        // twig v0. Before it, the shape every fixture base stands on had
        // nowhere to climb, so no gate could hold a foundation that a
        // hearth actually pays upkeep for: twig is never protected
        // (`deploy::upkeep_sweep`), and twig was all a foundation could
        // ever be. That made "a covered piece survives the sweep"
        // unprovable for the commonest shape in the world.
        b.pieces[5] = PieceDef {
            shape: SHAPE_FOUNDATION,
            material: MAT_STONE,
            hp: 200,
            n_costs: 1,
            costs: [(0, 5), (0, 0)],
        };
        // Row 6 is row 2's stone rung, and it costs the SECOND item on
        // purpose: rows 5 and 6 are then two graded pieces priced in two
        // different materials, which is what the per-material upkeep rule
        // needs to be about anything (`deploy::upkeep_sweep` charges row
        // by row, so a hearth stocked with one of them must protect one
        // and rot the other). Before twig v0 the twig-labelled rows did
        // that job; they cannot now, because twig is never upkept.
        b.pieces[6] = PieceDef {
            shape: SHAPE_FLOOR,
            material: MAT_STONE,
            hp: 200,
            n_costs: 1,
            costs: [(1, 4), (0, 0)],
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
    /// Which way the piece's SOFT side faces (hard/soft v0,
    /// `reference/BUILDING.md` §7b.5): 1 ⇒ soft toward **+axis** (+x for
    /// a west edge, +z for a north edge), 0 ⇒ soft toward −axis. Set once
    /// at placement — soft faces the placer, because you build from
    /// inside — and never moved (no rotate verb yet: inside the demolish
    /// window a wrong facing is a free re-place). Meaningful on edge
    /// shapes only; 0 elsewhere. On the wire (a client says which side
    /// you are on) and in `state_hash` (a swing's damage reads it).
    pub facing: u8,
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
    /// When each entry was placed, index-aligned to `entries` — the
    /// demolish window's clock (demolish v1, `limits.rs`
    /// `DEMOLISH_WINDOW_TICKS`).
    ///
    /// A parallel array rather than a field on `PieceRec`, and that is
    /// `Deploys::bag_ready`'s decision for `bag_ready`'s reason: the
    /// record is what the piece-sync packet mirrors, so eight bytes of
    /// sim-only timer on it would ride `PIECE_SYNC_BATCH`-deep in
    /// `EventMsg::PieceSync` and grow the client's event enum by bytes it
    /// can never read. A client draws a wall; it does not adjudicate one.
    /// Kept aligned by `insert` and by every removal, whose swap-remove
    /// moves both halves together.
    placed: Box<[u64; MAX_PIECES]>,
    /// Boxed for `placed`'s reason and then some: 8 192 records is ~98 KB,
    /// which `Pieces::new` was materialising in a frame. On 2026-08-08
    /// that frame plus dlmalloc's own tipped `World::new` past wasm32's
    /// 1 MiB shadow stack and `test_parity_wasm` died as an
    /// out-of-bounds read **inside the allocator** — the same trap as the
    /// lock store and the hearth crew, wearing a third disguise
    /// (`crate::boxed_array`).
    entries: Box<[PieceRec; MAX_PIECES]>,
    len: usize,
    cols: Box<crate::collide::ColIndex>,
    /// Bumped by every insert, removal and restore — the stamp
    /// `claim::ClaimCache` compares to know whether the base shapes it
    /// cached can have changed. **Derived-cache plumbing, not state**: it
    /// is never hashed and never saved, exactly like `cols`, and wall 5
    /// does not rest on its value — only on the cache it invalidates being
    /// a pure function of the pieces, which it is (`claim.rs`). It bumps
    /// on *any* store change rather than only footprint-changing ones
    /// (a second wall in an already-built cell bumps it too), because an
    /// extra rebuild costs a bounded walk and a missed one costs a wall
    /// the sweep still thinks is standing.
    gen: u64,
}

impl Pieces {
    pub fn new() -> Self {
        Self {
            placed: crate::boxed_array(0),
            entries: crate::boxed_array(PieceRec::default()),
            len: 0,
            cols: Box::new(crate::collide::ColIndex::new()),
            gen: 0,
        }
    }

    /// The store-change stamp (`gen`'s own doc). Read by
    /// `Deploys::refresh_claims` and nothing else.
    pub(crate) fn footprint_gen(&self) -> u64 {
        self.gen
    }

    /// The tick entry `i` was placed on — the demolish window's clock.
    /// Reads past the live half are 0, which is "placed at tick 0" and
    /// therefore long out of its window: the safe direction for a
    /// stale index, since it refuses rather than refunds.
    pub fn placed_at(&self, i: usize) -> u64 {
        if i >= self.len {
            return 0;
        }
        self.placed[i]
    }

    /// The live half of `placed`, index-aligned to `entries()` — read by
    /// `state_hash` (it is sim state) and by `worldsave`.
    pub fn placed(&self) -> &[u64] {
        &self.placed[..self.len]
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

    /// The store index of the piece at an address — what a write needs
    /// (`find` hands out a reference the borrow checker won't let a
    /// mutation follow).
    pub(crate) fn find_index(&self, cx: u16, cz: u16, level: u8, loc: u8) -> Option<usize> {
        self.entries[..self.len]
            .iter()
            .position(|p| p.cx == cx && p.cz == cz && p.level == level && p.loc == loc)
    }

    /// Re-row entry `i` in place (the upgrade verb's write). The address
    /// and the shape are unchanged by construction, so the column index
    /// and any door sealing that address are deliberately untouched —
    /// upgrading a doorway must not drop the door standing in it.
    fn set_row(&mut self, i: usize, row: u8, hp: u16) {
        self.entries[i].row = row;
        self.entries[i].hp = hp;
    }

    /// Append a record. False ⇒ store full (the caller refuses the
    /// placement; nothing is evicted). `shape` keeps the column index in
    /// lockstep — the caller has the baked row in hand.
    /// Put a piece straight into the store, bypassing every rule the
    /// `place` verb keeps. **Fixtures only** — `claim.rs`'s tests build a
    /// base to ask the privilege question of, and driving `place` to do it
    /// would make the fixture depend on the answer the test is checking.
    #[cfg(test)]
    pub(crate) fn insert_for_test(
        &mut self,
        cx: u16,
        cz: u16,
        level: u8,
        loc: u8,
        row: u8,
        bc: &BuildContent,
    ) {
        let rec = PieceRec {
            cx,
            cz,
            level,
            loc,
            row,
            facing: 0,
            hp: bc.pieces[row as usize].hp,
            uh: 0,
        };
        assert!(
            self.insert(rec, bc.pieces[row as usize].shape, 0),
            "the fixture overflowed the piece store"
        );
    }

    fn insert(&mut self, rec: PieceRec, shape: u8, tick: u64) -> bool {
        if self.len == MAX_PIECES {
            return false;
        }
        self.entries[self.len] = rec;
        self.placed[self.len] = tick;
        self.len += 1;
        self.cols.add(rec.cx, rec.cz, rec.level, rec.loc, shape);
        self.gen += 1;
        true
    }

    /// Swap-remove entry `i` (the decay sweep's removal; deploy.rs).
    pub(crate) fn remove_at(&mut self, i: usize, shape: u8) {
        let rec = self.entries[i];
        self.cols.del(rec.cx, rec.cz, rec.level, rec.loc, shape);
        self.len -= 1;
        self.entries[i] = self.entries[self.len];
        // Both halves move together, which is the whole contract of a
        // parallel array (`placed`'s own doc).
        self.placed[i] = self.placed[self.len];
        self.gen += 1;
    }

    /// Write entry `i`'s hp alone — the raid verb's write (deploy.rs
    /// `damage_piece`). The upkeep clock is deliberately untouched: taking
    /// damage is not paying rent.
    pub(crate) fn set_hp(&mut self, i: usize, hp: u16) {
        self.entries[i].hp = hp;
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

    /// Set or clear the solid-deployable nibble at (column, level) —
    /// `set_door`'s twin (deploy collision v0; deploy.rs owns when:
    /// placement, removal, and the load path's `World::rebuild_doors`).
    pub(crate) fn set_solid(&mut self, cx: u16, cz: u16, level: u8, arch: Option<u8>) {
        self.cols.set_solid(cx, cz, level, arch);
    }

    /// Replace the store from a decoded world save, rebuilding the column
    /// index from the records rather than reading one out of the file.
    ///
    /// **The index is derived state and this is why it is not persisted.**
    /// `cols` is a bitset view of exactly these records, maintained in
    /// lockstep by `insert`/`remove_at` and never hashed; a stored copy
    /// could disagree with the pieces it claims to describe, and the
    /// disagreement would present as a wall you can walk through — visible
    /// to a player, invisible to `state_hash`, and unreachable by any gate
    /// that compares two runs. Recomputing costs one pass over the pieces
    /// on the boot path.
    ///
    /// Doors are **not** restored here, and cannot be: a door's shut bit
    /// lives on a deployable record, not a piece. `World::rebuild_doors`
    /// runs after the deployables land (`worldsave.rs`).
    ///
    /// Boot-only, like everything on the load path.
    pub(crate) fn restore(&mut self, recs: &[PieceRec], placed: &[u64], bc: &BuildContent) {
        debug_assert_eq!(recs.len(), placed.len(), "placed must be index-aligned");
        self.gen += 1;
        self.cols.clear();
        self.len = recs.len().min(MAX_PIECES);
        self.entries[..self.len].copy_from_slice(&recs[..self.len]);
        self.placed[..self.len].copy_from_slice(&placed[..self.len]);
        for rec in &self.entries[..self.len] {
            // The row was range-checked by the decoder against
            // `piece_count`; this index is the one `worldsave.rs`
            // `BadContentRow` exists to make safe.
            let shape = bc.pieces[rec.row as usize].shape;
            self.cols.add(rec.cx, rec.cz, rec.level, rec.loc, shape);
        }
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
/// Placement measures build reach to it; a raid swing measures weapon
/// reach to the same point, so what you can build you can break.
///
/// **`pub` so the client can measure to the same corner.** It was
/// `pub(crate)`, which forced `web/src/interact.js` to keep a hand-written
/// `pieceAnchor` — and that file's own comment names the hazard exactly:
/// swap the two `half` terms and every byte-golden stays green while the
/// client measures reach to the wrong corner of the cell, refusing at a
/// distance the server would accept and reaching at one it will not. That is
/// `CLAUDE.md`'s positional-payload trap. One caller instead of two copies
/// closes it by construction rather than by a test that has to remember.
/// Exporting a pure function of three integers costs the sim nothing.
pub fn anchor(cx: u16, cz: u16, loc: u8) -> (f32, f32) {
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

/// Whether an edge shape occupies this address's facing at all — planes
/// and risers have no sides.
#[inline]
pub fn shape_has_facing(shape: u8) -> bool {
    matches!(
        shape,
        SHAPE_WALL | SHAPE_DOORWAY | SHAPE_WINDOW | SHAPE_FRAME
    )
}

/// The facing a placement gets: 1 when the placer stands on the edge's
/// **+axis** side, 0 otherwise — soft toward the builder, because a base
/// is built from inside (`PieceRec::facing`'s doc). Exactly on the plane
/// counts as −axis; the tie is impossible to stand on and the arm has to
/// pick something deterministic.
///
/// `pub` because the client needs the same answer twice: the ghost says
/// which side the soft face will land on before the key is pressed, and
/// the structure readout says which side you are looking at. A second
/// copy of this comparison is the positional-payload trap with a sign bit.
#[inline]
pub fn facing_of(loc: u8, cx: u16, cz: u16, px: f32, pz: f32) -> u8 {
    let plus = if loc == LOC_EDGE_W {
        px > cx as f32 * BUILD_CELL_M
    } else {
        pz > cz as f32 * BUILD_CELL_M
    };
    plus as u8
}

/// Is a toucher standing at (`px`, `pz`) on the piece's SOFT side? The
/// one comparison `combat::raid` prices a swing with and the client's
/// readout labels a wall with — the same function, so the label can
/// never disagree with the bill.
#[inline]
pub fn soft_side(rec: &PieceRec, px: f32, pz: f32) -> bool {
    facing_of(rec.loc, rec.cx, rec.cz, px, pz) == rec.facing
}

/// Will this ground hold a foundation? One definition — `place` refuses on
/// it, and a fixture that needs buildable ground finds it with it, so the
/// two can never drift apart.
pub fn foundation_terrain_ok(seed: u64, ax: f32, az: f32) -> bool {
    terrain::height(seed, ax, az) >= FOUNDATION_MIN_H_M
        && terrain::slope(seed, ax, az) < FOUNDATION_MAX_SLOPE
}

/// Whether `loc` is the kind of slot `shape` occupies.
fn loc_fits_shape(shape: u8, loc: u8) -> bool {
    match shape {
        SHAPE_FOUNDATION | SHAPE_FLOOR | SHAPE_ROOF => loc == LOC_PLANE,
        SHAPE_STAIRS => loc == LOC_RISER,
        SHAPE_WALL | SHAPE_DOORWAY | SHAPE_WINDOW | SHAPE_FRAME => {
            loc == LOC_EDGE_W || loc == LOC_EDGE_N
        }
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
        SHAPE_WALL | SHAPE_DOORWAY | SHAPE_WINDOW | SHAPE_FRAME => {
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

/// A grid address: (cx, cz, level, loc). What the collapse front carries.
type Addr = (u16, u16, u8, u8);

/// The most addresses `dependents` can name — a plane's five.
const MAX_DEPENDENTS: usize = 5;

/// The inverse of `supported()`: every address whose support test **reads**
/// `(cx, cz, level, loc)`. Change one of the two and you must change the
/// other; `collapse_matches_a_naive_fixed_point` is the gate that says so,
/// and it is the whole reason the pair sits in one place.
///
/// Read straight off `supported()` above, clause by clause:
/// - the stairs branch probes `plane_at(cx, cz, level)`, and both wall
///   branches probe `plane_at(.., level)` in the two cells the edge
///   adjoins — so a **plane** is read by the riser in its own cell and by
///   the four edges of its own cell, all at its own level;
/// - the floor/roof branch probes four edges at `level - 1`, and the wall
///   branch probes `edge_at(cx, cz, level - 1, loc)` — so an **edge** is
///   read by the planes one level up in the two cells it adjoins and by
///   the edge directly above it;
/// - nothing anywhere probes `LOC_RISER`, so a **riser** is read by
///   nothing: take the stairs out and the floor above still stands.
///
/// Foundations appear here as planes like any other; they are simply never
/// dropped themselves, because their own clause is unconditional.
fn dependents(cx: u16, cz: u16, level: u8, loc: u8, out: &mut [Addr; MAX_DEPENDENTS]) -> usize {
    match loc {
        LOC_PLANE => {
            out[0] = (cx, cz, level, LOC_RISER);
            out[1] = (cx, cz, level, LOC_EDGE_W);
            out[2] = (cx.saturating_add(1), cz, level, LOC_EDGE_W);
            out[3] = (cx, cz, level, LOC_EDGE_N);
            out[4] = (cx, cz.saturating_add(1), level, LOC_EDGE_N);
            5
        }
        LOC_EDGE_W | LOC_EDGE_N => {
            let up = level.saturating_add(1);
            if up >= MAX_BUILD_LEVELS as u8 {
                return 0; // nothing can be addressed above the top storey
            }
            out[0] = (cx, cz, up, LOC_PLANE);
            out[1] = (cx, cz, up, loc);
            let other = if loc == LOC_EDGE_W {
                cx.checked_sub(1).map(|x| (x, cz))
            } else {
                cz.checked_sub(1).map(|z| (cx, z))
            };
            match other {
                Some((bx, bz)) => {
                    out[2] = (bx, bz, up, LOC_PLANE);
                    3
                }
                None => 2, // the grid's low border: the edge adjoins one cell
            }
        }
        _ => 0,
    }
}

/// Whether any piece stands at the address — the O(1) column-index probe
/// (collide.rs), so a cascade only pays a store scan for a candidate that
/// actually exists. The index is kept in lockstep by `Pieces` itself and
/// `col_index_churn_matches_a_naive_shadow` gates that; here it is a
/// filter in front of `find_index`, never the answer.
fn occupied_at(pieces: &Pieces, cx: u16, cz: u16, level: u8, loc: u8) -> bool {
    if level >= MAX_BUILD_LEVELS as u8 {
        return false;
    }
    let m = pieces.cols().get(cx, cz);
    let field = match loc {
        LOC_PLANE => m.planes,
        LOC_RISER => m.stairs,
        LOC_EDGE_W => m.walls_w | m.doors_w | m.wins_w | m.frames_w,
        _ => m.walls_n | m.doors_n | m.wins_n | m.frames_n,
    };
    field & (1u8 << level) != 0
}

/// Does the piece at store index `i` still have what holds it up? The one
/// question `collapse_from` and `support_sweep` both ask, asked through
/// the same `supported()` that refused the placement — so a base can never
/// stand on a rule it could not have been built under.
pub(crate) fn stands(pieces: &Pieces, bc: &BuildContent, i: usize) -> bool {
    let r = pieces.entries()[i];
    let shape = bc.pieces[r.row as usize].shape;
    supported(pieces, shape, r.cx, r.cz, r.level, r.loc)
}

/// A piece at `from` has just been removed: re-evaluate what rested on it
/// and drop whatever no longer stands, breadth-first, until the front is
/// empty, `MAX_COLLAPSE_PIECES` is reached, or the tick's shared removal
/// budget runs out. Returns how many fell.
///
/// Two caps, and they are not the same cap. `MAX_COLLAPSE_PIECES` bounds
/// *this* cascade and sizes the stack array below. `budget` is the whole
/// tick's, spent by every removal path there is, and it is the one that
/// keeps the event ring from being filled by a tick that collapses many
/// bases at once (limits.rs `MAX_REMOVALS_PER_TICK`). Both defer to
/// `support_sweep` on the following ticks, so either running out costs
/// latency and never correctness.
///
/// Graph reachability over the piece store, not physics. Each removal
/// re-checks only the ≤ 5 addresses whose `supported()` test reads the one
/// that just went (`dependents`), so the work is proportional to what
/// actually falls and not to the size of the island. Removal goes through
/// `deploy::drop_piece` — the one removal path — so a collapsed floor
/// takes the box standing on it exactly the way a decayed one does, and
/// every piece announces itself with `EV_PIECE_REMOVED`.
///
/// The seed address is the piece that already went; it is never itself a
/// candidate, so this cannot re-enter on it. Nor can any address enter the
/// front twice: an address is pushed only when a piece is removed from it,
/// and after that `occupied_at` reads false there.
pub(crate) fn collapse_from(
    dc: &DeployContent,
    bc: &BuildContent,
    pieces: &mut Pieces,
    deploys: &mut Deploys,
    from: Addr,
    budget: &mut usize,
    events: &mut EventQueue,
) -> usize {
    // One slot per possible removal, plus the seed: `tail` advances only
    // where `fell` does, so the front can never outrun this.
    let mut front = [(0u16, 0u16, 0u8, 0u8); MAX_COLLAPSE_PIECES + 1];
    front[0] = from;
    let mut head = 0usize;
    let mut tail = 1usize;
    let mut fell = 0usize;
    let mut kids = [(0u16, 0u16, 0u8, 0u8); MAX_DEPENDENTS];
    while head < tail && fell < MAX_COLLAPSE_PIECES && *budget > 0 {
        let (cx, cz, level, loc) = front[head];
        head += 1;
        let n = dependents(cx, cz, level, loc, &mut kids);
        for &(kx, kz, kl, kloc) in kids.iter().take(n) {
            if !occupied_at(pieces, kx, kz, kl, kloc) {
                continue;
            }
            let Some(i) = pieces.find_index(kx, kz, kl, kloc) else {
                continue;
            };
            if stands(pieces, bc, i) {
                continue;
            }
            let shape = bc.pieces[pieces.entries()[i].row as usize].shape;
            crate::deploy::drop_piece(dc, pieces, deploys, i, shape, events);
            *budget -= 1;
            front[tail] = (kx, kz, kl, kloc);
            tail += 1;
            fell += 1;
            if fell == MAX_COLLAPSE_PIECES || *budget == 0 {
                break; // deferred to `support_sweep` (limits.rs)
            }
        }
    }
    fell
}

/// Walk the piece store on a cursor, `SUPPORT_SWEEP_PER_TICK` entries a
/// tick, and drop the first piece found standing on nothing — with its own
/// cascade. The backstop that makes `MAX_COLLAPSE_PIECES` a cap rather
/// than a promise: what one tick's cascade defers, a later tick finds.
///
/// At most one collapse a tick, for the same reason the cap exists: the
/// event ring is 256 and a removal no client hears is a piece drawn for
/// the rest of the session. On a shard where nothing is hanging this is a
/// bounded scan that removes nothing, which is the normal case — a piece
/// can only lose its support by another piece being removed, and that path
/// already collapses itself.
pub(crate) fn support_sweep(
    dc: &DeployContent,
    bc: &BuildContent,
    pieces: &mut Pieces,
    deploys: &mut Deploys,
    cursor: &mut u32,
    budget: &mut usize,
    events: &mut EventQueue,
) {
    let mut visits = 0usize;
    while visits < SUPPORT_SWEEP_PER_TICK && !pieces.is_empty() && *budget > 0 {
        visits += 1;
        let i = (*cursor as usize) % pieces.len();
        *cursor = ((i + 1) % pieces.len()) as u32;
        if stands(pieces, bc, i) {
            continue;
        }
        let r = pieces.entries()[i];
        let shape = bc.pieces[r.row as usize].shape;
        crate::deploy::drop_piece(dc, pieces, deploys, i, shape, events);
        *budget -= 1;
        collapse_from(
            dc,
            bc,
            pieces,
            deploys,
            (r.cx, r.cz, r.level, r.loc),
            budget,
            events,
        );
        // The swap-remove moved the last entry into `i`; resume there so a
        // long cascade cannot walk the cursor off the end of the store.
        *cursor = (i as u32).min(pieces.len().saturating_sub(1) as u32);
        return;
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
    // Everything enters the world as twig, and a finished grade is only
    // ever reached by `upgrade` (twig v0, `reference/BUILDING.md` §7b.4).
    // The client offers no other row, so in normal play this is
    // unreachable — it is here because the wire is not the client, and a
    // forged row is how a placement would otherwise skip the skeleton it
    // is supposed to cost.
    if def.material != MAT_TWIG {
        events.push(EV_BUILD_REFUSED, p.id, REFUSE_B_TIER, 0);
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
    if crate::claim::foreign_claim(pieces, deploys, ax, az, p.id) {
        events.push(EV_BUILD_REFUSED, p.id, REFUSE_B_CLAIM, 0);
        return;
    }
    if def.shape == SHAPE_FOUNDATION && !foundation_terrain_ok(seed, ax, az) {
        events.push(EV_BUILD_REFUSED, p.id, REFUSE_B_TERRAIN, 0);
        return;
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
        // Soft toward the builder (hard/soft v0). Planes and risers have
        // no sides and carry 0, so two replays cannot disagree about a
        // bit that means nothing.
        facing: if shape_has_facing(def.shape) {
            facing_of(loc, cx, cz, px, pz)
        } else {
            0
        },
        hp: def.hp,
        uh: (tick / UPKEEP_PERIOD_TICKS) as u16,
    };
    if !pieces.insert(rec, def.shape, tick) {
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

/// The baked row for a shape in a material, if the table holds one.
/// Linear over `MAX_PIECE_DEFS` (32) — the table is tiny and the scan
/// keeps the ladder a property of the content, not of row ordering.
fn row_of(bc: &BuildContent, shape: u8, material: u8) -> Option<u16> {
    bc.pieces
        .iter()
        .take(bc.piece_count as usize)
        .position(|d| d.hp != 0 && d.shape == shape && d.material == material)
        .map(|i| i as u16)
}

/// Apply one upgrade request (`Command::Upgrade`): the piece standing at
/// the address becomes the same shape in `material`, a rung further up the
/// wood → stone → metal ladder. `material` names the rung, not a step, so
/// wood → metal in one press is legal where the table holds the row; a
/// sideways or downward material refuses, because a raid must not be
/// answerable by re-skinning a wall in something cheaper.
///
/// Price is the target row's whole build cost — content says so
/// (`content/building.toml`: `cost` is the direct build cost *and* the
/// upgrade-into cost). Damage carries across as a fraction rather than
/// healing: a wall standing at half hp becomes a stone wall at half hp,
/// so an upgrade is never a free repair mid-raid. The upkeep clock is
/// left where it stood — an unpaid period is still owed.
///
/// The shape never changes, so collision never moves and the door in an
/// upgraded doorway keeps its leaf, its lock, and its seal.
#[allow(clippy::too_many_arguments)]
pub fn upgrade(
    bc: &BuildContent,
    deploys: &Deploys,
    pieces: &mut Pieces,
    p: &mut Player,
    cx: u16,
    cz: u16,
    level: u8,
    loc: u8,
    material: u8,
    events: &mut EventQueue,
) {
    let Some(i) = pieces.find_index(cx, cz, level, loc) else {
        events.push(EV_BUILD_REFUSED, p.id, REFUSE_B_SPOT, 0);
        return;
    };
    let rec = pieces.entries()[i];
    // A record's row was validated at placement; re-checking it here is
    // what keeps a content table swapped under a live store from indexing
    // out of bounds — and the inert row's `hp == 0` out of the damage
    // carry's divisor below (the sim never panics on state, wall 5).
    if rec.row as u16 >= bc.piece_count || bc.pieces[rec.row as usize].hp == 0 {
        events.push(EV_BUILD_REFUSED, p.id, REFUSE_B_PIECE, 0);
        return;
    }
    let cur = bc.pieces[rec.row as usize];
    let (ax, az) = anchor(cx, cz, loc);
    let px = p.body.qx as f32 * crate::movement::POS_XZ_Q;
    let pz = p.body.qz as f32 * crate::movement::POS_XZ_Q;
    let (dx, dz) = (ax - px, az - pz);
    if dx * dx + dz * dz > BUILD_REACH_M * BUILD_REACH_M {
        events.push(EV_BUILD_REFUSED, p.id, REFUSE_B_REACH, 0);
        return;
    }
    if crate::claim::foreign_claim(pieces, deploys, ax, az, p.id) {
        events.push(EV_BUILD_REFUSED, p.id, REFUSE_B_CLAIM, 0);
        return;
    }
    if material <= cur.material {
        events.push(EV_BUILD_REFUSED, p.id, REFUSE_B_TIER, 0);
        return;
    }
    let Some(row) = row_of(bc, cur.shape, material) else {
        events.push(EV_BUILD_REFUSED, p.id, REFUSE_B_TIER, 0);
        return;
    };
    let def = bc.pieces[row as usize];
    for &(item, units) in def.costs.iter().take(def.n_costs as usize) {
        if inv_count(&p.inv, item) < units as u32 {
            events.push(EV_BUILD_REFUSED, p.id, REFUSE_B_COST, 0);
            return;
        }
    }
    for &(item, units) in def.costs.iter().take(def.n_costs as usize) {
        inv_take(&mut p.inv, item, units as u32);
    }
    // Damage rides the ladder as a fraction, never below one hit of life
    // (cur.hp is nonzero: an inert row refused above).
    let carried = (rec.hp as u32 * def.hp as u32) / cur.hp as u32;
    let hp = carried.clamp(1, def.hp as u32) as u16;
    pieces.set_row(i, row as u8, hp);
    // Announced as a placement: the address now holds a different row,
    // which is exactly what EV_PIECE_PLACED says, and every mirror
    // downstream already upserts by address.
    events.push(
        EV_PIECE_PLACED,
        crate::gather::cell_key(cx, cz),
        ((level as u32) << 16) | ((loc as u32) << 8) | row as u32,
        0,
    );
}

/// What one cost row owes to buy back `missing` hp of a piece whose full
/// life is `max_hp`: the pro-rata share of that row, scaled by the content
/// percent, rounded up, and never free while any hp is missing.
///
/// `u64` deliberately. `units` and `missing` are both `u16`, so their
/// product times a percent tops out past `u32` even though shipped content
/// sits three orders below it — and a wrap here would sell a metal wall's
/// worth of repair for one wood.
fn repair_units(units: u16, missing: u16, max_hp: u16, repair_pct: u16) -> u32 {
    if units == 0 || missing == 0 || max_hp == 0 {
        return 0;
    }
    let num = units as u64 * missing as u64 * repair_pct as u64;
    let den = max_hp as u64 * 100;
    // Round up, then floor at one: a repair that restores hp for nothing
    // is the free heal the whole price exists to refuse.
    num.div_ceil(den).max(1) as u32
}

/// The wider of the two cost tables, so one price loop serves both stores.
const MAX_REPAIR_COSTS: usize = MAX_DEPLOY_COSTS;
const _: () = assert!(
    MAX_PIECE_COSTS <= MAX_REPAIR_COSTS,
    "repair copies a piece's cost rows into a deployable-width buffer"
);

/// Take a piece back down and refund it whole — **demolish v1**
/// (`reference/BUILDING.md` §6/§7 verb 9).
///
/// Three rules, and each is the reference's:
///
/// 1. **Only inside the grace window** (`limits.rs`
///    `DEMOLISH_WINDOW_TICKS`). The question this answers is *I put the
///    foundation in the wrong place*, asked by every player in their first
///    hour; a window answers it without making a crewmate able to
///    dismantle a base they were let into. Past it, a wall comes down by
///    explosives and nothing else.
/// 2. **Only where you may build.** The same `claim::foreign_claim` every
///    other build verb asks — one predicate, so the four cannot drift
///    about whose base is whose.
/// 3. **A full refund**, in the piece's own cost rows. It is an undo, not
///    a salvage: a fraction would make misplacing a foundation a tax, and
///    the reference does not charge one either. A refund a full pack
///    cannot hold falls at the demolisher's feet rather than ceasing to
///    exist (`spill`, 2026-08-14) — see the `spill` parameter for why the
///    feet and not the wall.
///
/// The removal is `drop_piece` + `collapse_from` — the **same** path decay
/// and a raid take, cascade included. A verb with its own removal is a
/// second chance for a floating floor to survive something that should
/// have brought it down.
///
/// `spill` is the caller's buffer, exactly `gather::swing`'s: this module
/// owns no container store and is not about to acquire one, so the
/// remainder goes out as data and `world.rs`'s single drain point stands
/// the bag up.
///
/// **The fall-point is the player's feet, and the wall is not a rival
/// answer.** `NOW.md` §0sp2 left this path open on the ground that a
/// demolished wall's refund belongs where the wall was; the arithmetic
/// says the two addresses cannot be far enough apart to matter. This verb
/// refuses beyond `BUILD_REACH_M` (below), `backpack::LOOT_REACH_M` **is**
/// `BUILD_REACH_M` (one `pub use`, not two knobs), and that is the radius
/// `spill_at` merges within — so a bag minted at the wall is always inside
/// the merge reach of the feet and the reverse. Picking the feet therefore
/// costs nothing a player could find, needs no piece-anchor geometry on
/// the drain, and keeps every give-back in one tick to one bag.
#[allow(clippy::too_many_arguments)]
pub fn demolish(
    dc: &DeployContent,
    bc: &BuildContent,
    gc: &crate::gather::GatherContent,
    pieces: &mut Pieces,
    deploys: &mut Deploys,
    p: &mut Player,
    tick: u64,
    cx: u16,
    cz: u16,
    level: u8,
    loc: u8,
    budget: &mut usize,
    events: &mut EventQueue,
    spill: &mut [crate::gather::ItemStack; crate::limits::INV_SLOTS],
) {
    let Some(i) = pieces.find_index(cx, cz, level, loc) else {
        events.push(EV_BUILD_REFUSED, p.id, REFUSE_B_SPOT, 0);
        return;
    };
    let (ax, az) = anchor(cx, cz, loc);
    let px = p.body.qx as f32 * crate::movement::POS_XZ_Q;
    let pz = p.body.qz as f32 * crate::movement::POS_XZ_Q;
    let (dx, dz) = (ax - px, az - pz);
    if dx * dx + dz * dz > BUILD_REACH_M * BUILD_REACH_M {
        events.push(EV_BUILD_REFUSED, p.id, REFUSE_B_REACH, 0);
        return;
    }
    if crate::claim::foreign_claim(pieces, deploys, ax, az, p.id) {
        events.push(EV_BUILD_REFUSED, p.id, REFUSE_B_CLAIM, 0);
        return;
    }
    // `saturating_sub` rather than a comparison: a hand-edited save could
    // carry a placement tick from the future, and the safe reading of one
    // is "the window is spent", not "forever".
    if tick.saturating_sub(pieces.placed_at(i)) > crate::limits::DEMOLISH_WINDOW_TICKS {
        events.push(EV_BUILD_REFUSED, p.id, REFUSE_B_WINDOW, 0);
        return;
    }
    if *budget == 0 {
        // The tick's removal allowance is spent. Refused rather than
        // deferred, unlike the decay sweep's: a sweep comes around again
        // on its own and a keypress does not, so telling the player
        // beats silently doing it later.
        events.push(EV_BUILD_REFUSED, p.id, REFUSE_B_FULL, 0);
        return;
    }
    let rec = pieces.entries()[i];
    let def = bc.pieces[rec.row as usize];
    // Refund before removal: `drop_piece` invalidates the index, and a
    // refund computed after it would be pricing a swapped-in neighbour.
    for &(item, count) in def.costs.iter().take(def.n_costs as usize) {
        crate::gather::inv_add_spilling(&mut p.inv, spill, item, count, gc.stack_max_of(item));
    }
    crate::deploy::drop_piece(dc, pieces, deploys, i, def.shape, events);
    *budget -= 1;
    collapse_from(
        dc,
        bc,
        pieces,
        deploys,
        (cx, cz, level, loc),
        budget,
        events,
    );
}

/// Buy a damaged structure back to its baked hp, priced in its own
/// materials. `deploy` picks the store the address names.
///
/// **The address alone is ambiguous and always was.** A door stands *in*
/// its doorway — `place_deploy`'s `PLACE_DOORWAY` arm requires the piece
/// at the identical `(cx, cz, level, loc)` — so both stores answer to one
/// address and a verb that guessed would mend the wrong thing. The wire
/// therefore carries a bit, which is not a new idea here: `EV_STRUCT_HIT`
/// settled the same ambiguity with `STRUCT_DEPLOY_BIT` when damage first
/// had to say which store it hit. Repair reaches exactly what damage
/// reaches, and says so the same way.
///
/// The shape is `upgrade`'s, checked in the same order, because the two
/// verbs answer the same question about the same address and a player who
/// learns one refusal has learned both. What differs is the ending: an
/// upgrade re-rows the address and carries damage across as a fraction
/// (`DECISIONS.md`, upgrade v0 — "never heals"), while a repair leaves the
/// row alone and pays cash for the hp. Neither touches the upkeep clock:
/// materials are not rent.
///
/// Pieces carry no owner and a deployable's is deliberately never on the
/// wire (`DeployRec::owner`), so "yours" means what it means everywhere
/// else in this file for both — no *foreign* hearth claims the anchor.
/// Outside every claim anyone may repair anyone's wall, the same door
/// `place` and `upgrade` already leave open.
///
/// Reach and claim are both taken at `anchor`, for both stores. That puts
/// a door and the doorway it stands in at one point as well as one
/// address, which is the whole property this verb depends on; it differs
/// by half a cell from the cell centre `place_deploy` reaches an edge
/// deployable at, and that is the build lane's origin function winning
/// inside a build-lane verb rather than an oversight.
#[allow(clippy::too_many_arguments)]
pub fn repair(
    bc: &BuildContent,
    dc: &DeployContent,
    deploys: &mut Deploys,
    pieces: &mut Pieces,
    p: &mut Player,
    deploy: bool,
    cx: u16,
    cz: u16,
    level: u8,
    loc: u8,
    events: &mut EventQueue,
) {
    let found = if deploy {
        deploys.find_index(cx, cz, level, loc)
    } else {
        pieces.find_index(cx, cz, level, loc)
    };
    let Some(i) = found else {
        events.push(EV_BUILD_REFUSED, p.id, REFUSE_B_SPOT, 0);
        return;
    };
    // Row, hp now, hp full, and the price rows — read out of whichever
    // table the bit named, then one body prices and pays for both.
    //
    // Re-validated for the same reason `upgrade` re-validates: a content
    // table swapped under a live store must refuse, never index out of
    // bounds, and never divide by the inert row's zero (wall 5).
    let mut costs = [(0u16, 0u16); MAX_REPAIR_COSTS];
    let (row, hp_now, hp_full, n_costs) = if deploy {
        let rec = deploys.entries()[i];
        if rec.row as u16 >= dc.def_count || dc.defs[rec.row as usize].hp == 0 {
            events.push(EV_BUILD_REFUSED, p.id, REFUSE_B_PIECE, 0);
            return;
        }
        let def = dc.defs[rec.row as usize];
        costs[..def.costs.len()].copy_from_slice(&def.costs);
        (rec.row, rec.hp, def.hp, def.n_costs)
    } else {
        let rec = pieces.entries()[i];
        if rec.row as u16 >= bc.piece_count || bc.pieces[rec.row as usize].hp == 0 {
            events.push(EV_BUILD_REFUSED, p.id, REFUSE_B_PIECE, 0);
            return;
        }
        let def = bc.pieces[rec.row as usize];
        costs[..def.costs.len()].copy_from_slice(&def.costs);
        (rec.row, rec.hp, def.hp, def.n_costs)
    };
    let (ax, az) = anchor(cx, cz, loc);
    let px = p.body.qx as f32 * crate::movement::POS_XZ_Q;
    let pz = p.body.qz as f32 * crate::movement::POS_XZ_Q;
    let (dx, dz) = (ax - px, az - pz);
    if dx * dx + dz * dz > BUILD_REACH_M * BUILD_REACH_M {
        events.push(EV_BUILD_REFUSED, p.id, REFUSE_B_REACH, 0);
        return;
    }
    if crate::claim::foreign_claim(pieces, deploys, ax, az, p.id) {
        events.push(EV_BUILD_REFUSED, p.id, REFUSE_B_CLAIM, 0);
        return;
    }
    // Nothing to buy. `saturating_sub` also answers the swapped-table case
    // where the store holds more hp than the row now allows — refusing is
    // right there, because trimming a stranger's wall is not this verb.
    // A zero percent means the table was never baked, and the alternative
    // to refusing is healing free.
    let missing = hp_full.saturating_sub(hp_now);
    if missing == 0 || bc.repair_pct == 0 {
        events.push(EV_BUILD_REFUSED, p.id, REFUSE_B_INTACT, 0);
        return;
    }
    // A row with no price rows would fall straight through both loops
    // below, take nothing, and mend anyway. That is the free heal the
    // price exists to refuse, so it is refused by name rather than by the
    // loops happening to be non-empty.
    let n_costs = (n_costs as usize).min(costs.len());
    if n_costs == 0 {
        events.push(EV_BUILD_REFUSED, p.id, REFUSE_B_UNPRICED, 0);
        return;
    }
    // Check every row, then take every row — `place`'s split, for
    // `place`'s reason. A half-paid repair leaves the client's mirror and
    // the server's store disagreeing about an inventory, which is the
    // divergence class `CLAUDE.md`'s trap list names one verb over.
    for &(item, units) in costs.iter().take(n_costs) {
        if inv_count(&p.inv, item) < repair_units(units, missing, hp_full, bc.repair_pct) {
            events.push(EV_BUILD_REFUSED, p.id, REFUSE_B_COST, 0);
            return;
        }
    }
    for &(item, units) in costs.iter().take(n_costs) {
        inv_take(
            &mut p.inv,
            item,
            repair_units(units, missing, hp_full, bc.repair_pct),
        );
    }
    // The wall: a repaired structure *is* its baked row's hp and never a
    // point more. Written as an assignment rather than an add-and-clamp so
    // there is no arithmetic between the store and the ceiling to get
    // wrong.
    if deploy {
        deploys.set_hp(i, hp_full);
    } else {
        pieces.set_hp(i, hp_full);
    }
    events.push(
        EV_PIECE_REPAIRED,
        crate::gather::cell_key(cx, cz),
        // The same bit in the same place `EV_STRUCT_HIT` puts it, because
        // it means the same thing: level, loc and row are 8-bit fields
        // below it, so bit 24 is the first free one.
        if deploy { STRUCT_DEPLOY_BIT } else { 0 }
            | ((level as u32) << 16)
            | ((loc as u32) << 8)
            | row as u32,
        ((missing as u32) << 16) | hp_full as u32,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::deploy::UPKEEP_PERIOD_TICKS;
    use crate::gather::ItemStack;
    use crate::limits::{INV_SLOTS, MAX_REMOVALS_PER_TICK};

    /// One tick's structural removal budget, as `World::tick` hands it
    /// out. These fixtures collapse or raid one structure at a time and
    /// never approach it — the bound itself is asserted by
    /// `a_tick_that_collapses_many_bases_at_once_drops_no_removal` and
    /// `many_raiders_in_one_tick_share_one_budget_and_the_last_wall_survives`.
    fn tick_budget() -> usize {
        MAX_REMOVALS_PER_TICK
    }

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

    /// Catalogue v1's two shapes hold the wall's slots and the wall's
    /// rules: an edge loc, wall support, a refusal on a plane loc — and
    /// the upgrade verb climbs them like any other shape.
    #[test]
    fn window_and_frame_place_as_edges_and_climb_the_ladder() {
        let mut bc = BuildContent::probe_fixture();
        // Rows 7..10: twig window, twig frame, stone window — the probe
        // fixture's shape, extended rather than replaced.
        bc.piece_count = 10;
        bc.pieces[7] = PieceDef {
            shape: SHAPE_WINDOW,
            material: MAT_TWIG,
            hp: 100,
            n_costs: 1,
            costs: [(0, 3), (0, 0)],
        };
        bc.pieces[8] = PieceDef {
            shape: SHAPE_FRAME,
            material: MAT_TWIG,
            hp: 100,
            n_costs: 1,
            costs: [(0, 3), (0, 0)],
        };
        bc.pieces[9] = PieceDef {
            shape: SHAPE_WINDOW,
            material: MAT_STONE,
            hp: 200,
            n_costs: 1,
            costs: [(1, 4), (0, 0)],
        };
        let mut pieces = Pieces::new();
        let nod = Deploys::new();
        let mut ev = EventQueue::default();
        let mut p = player_at_cell_center(&[(0, 40), (1, 10)]);

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
        // A window on a plane loc is a spot refusal, not a support one.
        place(
            SEED,
            &bc,
            &nod,
            &mut pieces,
            &mut p,
            0,
            7,
            CX,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(
            last(&ev),
            (crate::world::EV_BUILD_REFUSED, 7, REFUSE_B_SPOT)
        );
        // On the edge beside the foundation, both place.
        place(
            SEED,
            &bc,
            &nod,
            &mut pieces,
            &mut p,
            0,
            7,
            CX,
            CZ,
            0,
            LOC_EDGE_W,
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
            8,
            CX,
            CZ,
            0,
            LOC_EDGE_N,
            &mut ev,
        );
        assert_eq!(last(&ev).0, crate::world::EV_PIECE_PLACED);
        assert_eq!(pieces.cols().get(CX, CZ).wins_w, 1, "window in the index");
        assert_eq!(pieces.cols().get(CX, CZ).frames_n, 1, "frame in the index");

        // The hammer climbs the window to stone; the frame has no rung
        // above twig in this table and refuses by name.
        upgrade(
            &bc,
            &nod,
            &mut pieces,
            &mut p,
            CX,
            CZ,
            0,
            LOC_EDGE_W,
            MAT_STONE,
            &mut ev,
        );
        assert_eq!(last(&ev).0, crate::world::EV_PIECE_PLACED);
        let w = pieces.find(CX, CZ, 0, LOC_EDGE_W).unwrap();
        assert_eq!(w.row, 9, "the window re-rowed to its stone rung");
        upgrade(
            &bc,
            &nod,
            &mut pieces,
            &mut p,
            CX,
            CZ,
            0,
            LOC_EDGE_N,
            MAT_STONE,
            &mut ev,
        );
        assert_eq!(
            last(&ev),
            (crate::world::EV_BUILD_REFUSED, 7, REFUSE_B_TIER)
        );
    }

    /// Hard/soft v0: a placement's soft side faces the builder, on both
    /// edge axes; planes carry no facing; and `soft_side` answers the
    /// builder's own stance as soft — the label `combat::raid` prices by.
    #[test]
    fn facing_is_set_toward_the_builder_and_only_on_edges() {
        let bc = BuildContent::probe_fixture();
        let mut pieces = Pieces::new();
        let nod = Deploys::new();
        let mut ev = EventQueue::default();
        // The builder stands at the cell centre: east of the west edge,
        // south of the north edge.
        let mut p = player_at_cell_center(&[(0, 50), (1, 10)]);

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
            CX,
            CZ,
            0,
            LOC_EDGE_W,
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
            CX,
            CZ,
            0,
            LOC_EDGE_N,
            &mut ev,
        );

        let plane = pieces.find(CX, CZ, 0, LOC_PLANE).unwrap();
        assert_eq!(plane.facing, 0, "a plane has no sides");
        let w = *pieces.find(CX, CZ, 0, LOC_EDGE_W).unwrap();
        assert_eq!(w.facing, 1, "west edge: builder east => soft east");
        let n = *pieces.find(CX, CZ, 0, LOC_EDGE_N).unwrap();
        assert_eq!(n.facing, 1, "north edge: builder south => soft south");

        // The builder's own stance is the soft side of both walls; a
        // stranger across the edge is on the hard one.
        let px = (CX as f32 + 0.5) * BUILD_CELL_M;
        let pz = (CZ as f32 + 0.5) * BUILD_CELL_M;
        assert!(soft_side(&w, px, pz));
        assert!(soft_side(&n, px, pz));
        assert!(!soft_side(&w, px - BUILD_CELL_M, pz), "west of the wall");
        assert!(!soft_side(&n, px, pz - BUILD_CELL_M), "north of the wall");
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
                    facing: 0,
                    hp: 1,
                    uh: 0,
                },
                SHAPE_FOUNDATION,
                0
            ));
        }
        assert!(!pieces.insert(PieceRec::default(), SHAPE_FOUNDATION, 0));
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

    /// Stand a wood wall at the smoke cell's west edge and hand back the
    /// player who built it (foundation + wall paid from `items`).
    fn walled(items: &[(u16, u16)]) -> (BuildContent, Pieces, Deploys, EventQueue, Player) {
        let bc = BuildContent::probe_fixture();
        let mut pieces = Pieces::new();
        let nod = Deploys::new();
        let mut ev = EventQueue::default();
        let mut p = player_at_cell_center(items);
        for (row, loc) in [(0u16, LOC_PLANE), (1u16, LOC_EDGE_W)] {
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
                0,
                loc,
                &mut ev,
            );
            assert_eq!(last(&ev).0, crate::world::EV_PIECE_PLACED);
        }
        (bc, pieces, nod, ev, p)
    }

    /// `repair`'s doc comment owns a half-cell asymmetry — reach is taken at
    /// `anchor`, which is where damage is taken, while `place_deploy` takes it
    /// at the cell centre. It was documented and deliberate and it was
    /// **ungated**, so a player who could place a door from a spot and not
    /// mend it from the same spot reddened nothing
    /// (`pass-20260805-074623-04-judge.md`, ranked fix 2).
    ///
    /// This pins the relation rather than the behaviour: for a plane the two
    /// verbs measure to one identical point, and for an edge they are exactly
    /// half a cell apart and no more. Whichever way a later pass resolves it —
    /// move `repair` onto the centre, move `place_deploy` onto the anchor, or
    /// keep both — it goes red here first and the new relation has to be
    /// written down. The witness in the third block is the concrete cost:
    /// 1.5 m of the 5 m reach, on the far side.
    #[test]
    fn repair_and_placement_measure_an_edge_from_points_half_a_cell_apart() {
        let centre = crate::deploy::cell_center(CX, CZ);

        for loc in [LOC_PLANE, LOC_RISER] {
            assert_eq!(
                anchor(CX, CZ, loc),
                centre,
                "loc {loc} sits on the plane: placement and repair must measure \
                 reach to one point, so a deployed box is mendable from every \
                 spot it was placeable from"
            );
        }

        let half = BUILD_CELL_M * 0.5;
        for loc in [LOC_EDGE_W, LOC_EDGE_N] {
            let (ax, az) = anchor(CX, CZ, loc);
            let (dx, dz) = (ax - centre.0, az - centre.1);
            assert_eq!(
                dx * dx + dz * dz,
                half * half,
                "loc {loc}: the two reach centres are {} m apart, not the half \
                 cell repair's doc comment claims. The offset is the whole of \
                 the asymmetry — if it moved, the doc is now wrong",
                // squared, so this only formats on failure
                dx * dx + dz * dz
            );
        }

        // The witness: stand at the far edge of placement reach, on the side
        // the anchor is offset away from. Placement is exactly in range;
        // repair is half a cell out of it. Both verbs compare planar squared
        // distance against BUILD_REACH_M, so this is their arithmetic.
        let (ax, az) = anchor(CX, CZ, LOC_EDGE_W);
        let (px, pz) = (centre.0 + BUILD_REACH_M, centre.1);
        let place_d2 = (px - centre.0) * (px - centre.0) + (pz - centre.1) * (pz - centre.1);
        let repair_d2 = (px - ax) * (px - ax) + (pz - az) * (pz - az);
        assert!(
            place_d2 <= BUILD_REACH_M * BUILD_REACH_M,
            "the witness must be inside placement reach or it proves nothing"
        );
        assert!(
            repair_d2 > BUILD_REACH_M * BUILD_REACH_M,
            "a spot at the far edge of placement reach is out of repair reach \
             by half a cell — if this is no longer true the asymmetry is gone \
             and repair's doc comment should stop claiming it"
        );
    }

    /// The wall the whole verb exists to keep: a repaired piece stands at
    /// its baked row's hp and never a point more.
    ///
    /// Three damage levels rather than one, because "restore to full" and
    /// "add back the difference" agree on every case except the ones that
    /// overflow — and the second is the shape that would eventually be
    /// written here by someone adding a partial repair. The last case is
    /// the one no arithmetic reaches: a store holding *more* hp than the
    /// row now allows, which is what a content table swapped under a live
    /// world looks like. It refuses rather than trimming, because
    /// shortening a stranger's wall is not this verb, and it refuses
    /// rather than paying, because there is nothing to buy.
    #[test]
    fn a_repaired_piece_never_exceeds_its_baked_hp() {
        let full = BuildContent::probe_fixture().pieces[1].hp;
        for standing in [1u16, 40, full - 1] {
            let (bc, mut pieces, mut nod, mut ev, mut p) = walled(&[(0, 100)]);
            let i = pieces.find_index(CX, CZ, 0, LOC_EDGE_W).unwrap();
            pieces.set_upkeep(i, standing, 3);
            repair(
                &bc,
                &DeployContent::EMPTY,
                &mut nod,
                &mut pieces,
                &mut p,
                false,
                CX,
                CZ,
                0,
                LOC_EDGE_W,
                &mut ev,
            );
            let rec = *pieces.find(CX, CZ, 0, LOC_EDGE_W).unwrap();
            assert_eq!(
                rec.hp, full,
                "a wall repaired from {standing} stands at its baked hp, \
                 not at {} ",
                rec.hp
            );
            assert_eq!(
                rec.uh, 3,
                "materials are not rent: the upkeep clock does not move"
            );
            assert_eq!(rec.row, 1, "a repair is not an upgrade");
            // And a second press buys nothing, at no charge.
            let paid = inv_count(&p.inv, 0);
            repair(
                &bc,
                &DeployContent::EMPTY,
                &mut nod,
                &mut pieces,
                &mut p,
                false,
                CX,
                CZ,
                0,
                LOC_EDGE_W,
                &mut ev,
            );
            assert_eq!(
                last(&ev),
                (crate::world::EV_BUILD_REFUSED, 7, REFUSE_B_INTACT)
            );
            assert_eq!(inv_count(&p.inv, 0), paid, "an intact piece is free");
        }

        let (bc, mut pieces, mut nod, mut ev, mut p) = walled(&[(0, 100)]);
        let i = pieces.find_index(CX, CZ, 0, LOC_EDGE_W).unwrap();
        pieces.set_hp(i, full + 50);
        let before = inv_count(&p.inv, 0);
        repair(
            &bc,
            &DeployContent::EMPTY,
            &mut nod,
            &mut pieces,
            &mut p,
            false,
            CX,
            CZ,
            0,
            LOC_EDGE_W,
            &mut ev,
        );
        assert_eq!(
            last(&ev),
            (crate::world::EV_BUILD_REFUSED, 7, REFUSE_B_INTACT)
        );
        assert_eq!(
            pieces.find(CX, CZ, 0, LOC_EDGE_W).unwrap().hp,
            full + 50,
            "an over-hp record is left alone, not trimmed to the row"
        );
        assert_eq!(inv_count(&p.inv, 0), before, "and nothing was charged");
    }

    /// The price is the damage's own worth, rounded up, floored at one —
    /// and unpayable is a refusal that costs nothing.
    ///
    /// The fixture wall is 100 hp for 3 wood, so a 60 hp hole is 1.8 wood
    /// and must round to 2: rounding down would sell the last fifth of
    /// every repair free, and repeated presses would then heal a base for
    /// nothing. The one-hp scratch is the same argument at the bottom of
    /// the range, where rounding down reaches zero outright.
    #[test]
    fn repair_is_priced_pro_rata_rounded_up_and_never_free() {
        // 100 wood in, 5 for the foundation and 3 for the wall: 92 left.
        let (bc, mut pieces, mut nod, mut ev, mut p) = walled(&[(0, 100)]);
        let i = pieces.find_index(CX, CZ, 0, LOC_EDGE_W).unwrap();
        pieces.set_hp(i, 40);
        repair(
            &bc,
            &DeployContent::EMPTY,
            &mut nod,
            &mut pieces,
            &mut p,
            false,
            CX,
            CZ,
            0,
            LOC_EDGE_W,
            &mut ev,
        );
        assert_eq!(
            inv_count(&p.inv, 0),
            90,
            "60 hp of a 100 hp / 3 wood wall is 1.8 wood, charged as 2"
        );

        pieces.set_hp(i, 99);
        repair(
            &bc,
            &DeployContent::EMPTY,
            &mut nod,
            &mut pieces,
            &mut p,
            false,
            CX,
            CZ,
            0,
            LOC_EDGE_W,
            &mut ev,
        );
        assert_eq!(
            inv_count(&p.inv, 0),
            89,
            "a one-hp scratch is 0.03 wood and is charged as 1 — free \
             repair is the whole failure mode the price exists to refuse"
        );

        // Exactly enough wood to build, and none to mend with.
        let (bc, mut pieces, mut nod, mut ev, mut p) = walled(&[(0, 8)]);
        let i = pieces.find_index(CX, CZ, 0, LOC_EDGE_W).unwrap();
        pieces.set_hp(i, 40);
        repair(
            &bc,
            &DeployContent::EMPTY,
            &mut nod,
            &mut pieces,
            &mut p,
            false,
            CX,
            CZ,
            0,
            LOC_EDGE_W,
            &mut ev,
        );
        assert_eq!(
            last(&ev),
            (crate::world::EV_BUILD_REFUSED, 7, REFUSE_B_COST)
        );
        assert_eq!(
            pieces.find(CX, CZ, 0, LOC_EDGE_W).unwrap().hp,
            40,
            "a refused repair heals nothing"
        );
        assert_eq!(inv_count(&p.inv, 0), 0, "and takes nothing");
    }

    /// An unbaked table refuses rather than healing free.
    ///
    /// `BuildContent::EMPTY` carries `repair_pct == 0`, and content
    /// validation pins the shipped value to 1..=100 — so the only way a
    /// zero reaches here is a table nobody baked. The dangerous reading of
    /// a zero percent is "costs nothing", which is a free heal for every
    /// wall on the shard; the safe one is "no price is known", which is a
    /// refusal.
    #[test]
    fn an_unpriced_table_refuses_the_repair_rather_than_giving_it_away() {
        let (mut bc, mut pieces, mut nod, mut ev, mut p) = walled(&[(0, 100)]);
        let i = pieces.find_index(CX, CZ, 0, LOC_EDGE_W).unwrap();
        pieces.set_hp(i, 40);
        bc.repair_pct = 0;
        let before = inv_count(&p.inv, 0);
        repair(
            &bc,
            &DeployContent::EMPTY,
            &mut nod,
            &mut pieces,
            &mut p,
            false,
            CX,
            CZ,
            0,
            LOC_EDGE_W,
            &mut ev,
        );
        assert_eq!(
            last(&ev),
            (crate::world::EV_BUILD_REFUSED, 7, REFUSE_B_INTACT)
        );
        assert_eq!(pieces.find(CX, CZ, 0, LOC_EDGE_W).unwrap().hp, 40);
        assert_eq!(inv_count(&p.inv, 0), before);
    }

    /// Privilege, the same rule `place` and `upgrade` already carry: a
    /// foreign hearth's claim refuses, and its owner is unaffected.
    ///
    /// Worth its own case rather than trusting the shared shape, because
    /// this is the verb where getting it wrong is *generous* — a repair
    /// refused wrongly is an annoyance, a repair allowed wrongly lets a
    /// raider heal the wall they are standing outside of, which is not a
    /// bug anyone would think to look for.
    /// Demolish v1: inside the window it is a full undo, outside it the
    /// wall stays up.
    #[test]
    fn demolish_refunds_whole_inside_the_window_and_refuses_outside_it() {
        let bc = BuildContent::probe_fixture();
        let dc = DeployContent::probe_fixture();
        let gc = crate::gather::GatherContent::probe_fixture();
        let mut pieces = Pieces::new();
        let mut deploys = Deploys::new();
        let mut ev = EventQueue::default();
        let mut p = player_at_cell_center(&[(0, 99), (1, 99)]);
        let mut budget = MAX_REMOVALS_PER_TICK;

        let before = crate::craft::inv_count(&p.inv, 0);
        place(
            SEED,
            &bc,
            &deploys,
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
        assert_eq!(pieces.len(), 1, "the foundation stands");
        let paid = before - crate::craft::inv_count(&p.inv, 0);
        assert!(paid > 0, "the fixture must actually charge for it");

        // Inside the window: the piece comes down and the whole price
        // comes back. An undo, not a salvage.
        demolish(
            &dc,
            &bc,
            &gc,
            &mut pieces,
            &mut deploys,
            &mut p,
            10,
            CX,
            CZ,
            0,
            LOC_PLANE,
            &mut budget,
            &mut ev,
            &mut [ItemStack::default(); INV_SLOTS],
        );
        assert_eq!(pieces.len(), 0, "the foundation came down");
        assert_eq!(
            crate::craft::inv_count(&p.inv, 0),
            before,
            "a demolish inside the window refunds WHOLE — a fraction would \
             make misplacing a foundation a tax"
        );

        // Place another and let the window lapse.
        place(
            SEED,
            &bc,
            &deploys,
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
        let held = crate::craft::inv_count(&p.inv, 0);
        let late = crate::limits::DEMOLISH_WINDOW_TICKS + 1;
        demolish(
            &dc,
            &bc,
            &gc,
            &mut pieces,
            &mut deploys,
            &mut p,
            late,
            CX,
            CZ,
            0,
            LOC_PLANE,
            &mut budget,
            &mut ev,
            &mut [ItemStack::default(); INV_SLOTS],
        );
        assert_eq!(
            (last(&ev).0, last(&ev).2),
            (crate::world::EV_BUILD_REFUSED, REFUSE_B_WINDOW),
            "past the window a wall comes down by explosives and nothing else"
        );
        assert_eq!(pieces.len(), 1, "and it is still standing");
        assert_eq!(crate::craft::inv_count(&p.inv, 0), held, "and cost nothing");

        // The window is measured from THIS piece's placement, not from the
        // first one's — the parallel array's whole job.
        demolish(
            &dc,
            &bc,
            &gc,
            &mut pieces,
            &mut deploys,
            &mut p,
            0,
            CX,
            CZ,
            0,
            LOC_PLANE,
            &mut budget,
            &mut ev,
            &mut [ItemStack::default(); INV_SLOTS],
        );
        assert_eq!(pieces.len(), 0, "the second piece has its own clock");
    }

    /// The two shape refusals, and the claim — demolish asks the same
    /// predicate every other build verb does.
    #[test]
    fn demolish_bounces_on_an_empty_address_reach_and_a_foreign_claim() {
        let bc = BuildContent::probe_fixture();
        let dc = DeployContent::probe_fixture();
        let gc = crate::gather::GatherContent::probe_fixture();
        let mut pieces = Pieces::new();
        let mut deploys = Deploys::new();
        let mut ev = EventQueue::default();
        let mut p = player_at_cell_center(&[(0, 99), (1, 99), (2, 2)]);
        let mut budget = MAX_REMOVALS_PER_TICK;

        demolish(
            &dc,
            &bc,
            &gc,
            &mut pieces,
            &mut deploys,
            &mut p,
            0,
            CX,
            CZ,
            0,
            LOC_PLANE,
            &mut budget,
            &mut ev,
            &mut [ItemStack::default(); INV_SLOTS],
        );
        assert_eq!(last(&ev).2, REFUSE_B_SPOT, "nothing at that address");
        let _ = &mut ev;

        place(
            SEED,
            &bc,
            &deploys,
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
        let mut far = Player {
            id: 8,
            active: true,
            body: Body::at(
                SEED,
                (CX as f32 + 7.5) * BUILD_CELL_M,
                (CZ as f32 + 0.5) * BUILD_CELL_M,
            ),
            ..Player::default()
        };
        demolish(
            &dc,
            &bc,
            &gc,
            &mut pieces,
            &mut deploys,
            &mut far,
            0,
            CX,
            CZ,
            0,
            LOC_PLANE,
            &mut budget,
            &mut ev,
            &mut [ItemStack::default(); INV_SLOTS],
        );
        assert_eq!(last(&ev).2, REFUSE_B_REACH, "demolish has the build reach");

        // Somebody else's claim refuses it, which is what stops a
        // passer-by undoing a fresh base inside its own window.
        crate::deploy::place_deploy(
            SEED,
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut p,
            0,
            0,
            CX,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
        );
        let mut stranger = player_at_cell_center(&[]);
        stranger.id = 9;
        demolish(
            &dc,
            &bc,
            &gc,
            &mut pieces,
            &mut deploys,
            &mut stranger,
            0,
            CX,
            CZ,
            0,
            LOC_PLANE,
            &mut budget,
            &mut ev,
            &mut [ItemStack::default(); INV_SLOTS],
        );
        assert_eq!(
            (last(&ev).0, last(&ev).2),
            (crate::world::EV_BUILD_REFUSED, REFUSE_B_CLAIM),
            "a passer-by cannot undo a base they were never let into"
        );
        assert_eq!(pieces.len(), 1);
    }

    #[test]
    fn repair_refuses_under_a_foreign_claim() {
        let bc = BuildContent::probe_fixture();
        let dc = DeployContent::probe_fixture();
        let mut pieces = Pieces::new();
        let mut deploys = Deploys::new();
        let mut ev = EventQueue::default();
        let mut owner = player_at_cell_center(&[(0, 100), (1, 100), (2, 100), (3, 100)]);
        for (row, loc) in [(0u16, LOC_PLANE), (1u16, LOC_EDGE_W)] {
            place(
                SEED,
                &bc,
                &deploys,
                &mut pieces,
                &mut owner,
                0,
                row,
                CX,
                CZ,
                0,
                loc,
                &mut ev,
            );
            assert_eq!(last(&ev).0, crate::world::EV_PIECE_PLACED);
        }
        let hearth = (0..dc.def_count)
            .find(|&r| dc.defs[r as usize].arch == crate::deploy::ARCH_HEARTH)
            .expect("fixture holds a hearth");
        crate::deploy::place_deploy(
            SEED,
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut owner,
            0,
            hearth,
            CX,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(last(&ev).0, crate::world::EV_DEPLOY_PLACED, "hearth stands");
        let i = pieces.find_index(CX, CZ, 0, LOC_EDGE_W).unwrap();
        pieces.set_hp(i, 40);

        let mut stranger = player_at_cell_center(&[(0, 100)]);
        stranger.id = 9;
        repair(
            &bc,
            &DeployContent::EMPTY,
            &mut deploys,
            &mut pieces,
            &mut stranger,
            false,
            CX,
            CZ,
            0,
            LOC_EDGE_W,
            &mut ev,
        );
        assert_eq!(
            last(&ev),
            (crate::world::EV_BUILD_REFUSED, 9, REFUSE_B_CLAIM)
        );
        assert_eq!(
            pieces.find(CX, CZ, 0, LOC_EDGE_W).unwrap().hp,
            40,
            "a stranger inside someone's claim cannot mend their wall"
        );
        assert_eq!(inv_count(&stranger.inv, 0), 100, "and pays nothing to try");

        repair(
            &bc,
            &DeployContent::EMPTY,
            &mut deploys,
            &mut pieces,
            &mut owner,
            false,
            CX,
            CZ,
            0,
            LOC_EDGE_W,
            &mut ev,
        );
        assert_eq!(
            pieces.find(CX, CZ, 0, LOC_EDGE_W).unwrap().hp,
            bc.pieces[1].hp,
            "the hearth's owner mends their own wall"
        );
    }

    #[test]
    fn upgrade_pays_re_rows_in_place_and_carries_damage() {
        let (bc, mut pieces, nod, mut ev, mut p) = walled(&[(0, 20), (1, 10)]);
        let i = pieces.find_index(CX, CZ, 0, LOC_EDGE_W).unwrap();
        // Standing damage: half the wall's life is gone (the decay sweep
        // is what does this in the live sim).
        pieces.set_upkeep(i, 50, 0);
        let before = pieces.len();

        upgrade(
            &bc,
            &nod,
            &mut pieces,
            &mut p,
            CX,
            CZ,
            0,
            LOC_EDGE_W,
            MAT_STONE,
            &mut ev,
        );
        assert_eq!(
            last(&ev),
            (
                crate::world::EV_PIECE_PLACED,
                crate::gather::cell_key(CX, CZ),
                ((LOC_EDGE_W as u32) << 8) | 4
            ),
            "the upgrade announces the address's new row"
        );
        assert_eq!(inv_count(&p.inv, 1), 6, "the stone rung's whole cost paid");
        assert_eq!(inv_count(&p.inv, 0), 12, "the wood cost was not re-paid");
        assert_eq!(pieces.len(), before, "an upgrade is not a placement");
        let rec = *pieces.find(CX, CZ, 0, LOC_EDGE_W).unwrap();
        assert_eq!(rec.row, 4, "the address holds the stone row now");
        assert_eq!(rec.hp, 100, "half a 100 hp wall is half a 200 hp wall");
        assert_eq!(rec.uh, 0, "the upkeep clock is not reset by an upgrade");
    }

    /// **A placement is twig or it is refused** (twig v0,
    /// `reference/BUILDING.md` §7b.4). The rule that makes the skeleton
    /// cost something, and the one a forged row would otherwise skip: the
    /// client offers no other rung, so nothing but the wire can ask for
    /// one, and this is what answers it.
    #[test]
    fn a_placement_is_twig_or_it_is_refused() {
        let (bc, mut pieces, nod, mut ev, mut p) = walled(&[(0, 100), (1, 100)]);
        let before_pieces = pieces.len();
        let (before_0, before_1) = (inv_count(&p.inv, 0), inv_count(&p.inv, 1));

        // Row 4 is the stone wall — a real row, a legal address, a
        // supported spot, an affordable price. Only the rung is wrong.
        assert_eq!(bc.pieces[4].material, MAT_STONE);
        place(
            SEED,
            &bc,
            &nod,
            &mut pieces,
            &mut p,
            0,
            4,
            CX,
            CZ,
            0,
            LOC_EDGE_N,
            &mut ev,
        );
        assert_eq!(
            last(&ev),
            (crate::world::EV_BUILD_REFUSED, 7, REFUSE_B_TIER),
            "a finished grade may only be reached by upgrading into it"
        );
        assert_eq!(pieces.len(), before_pieces, "and nothing was placed");
        assert_eq!(
            (inv_count(&p.inv, 0), inv_count(&p.inv, 1)),
            (before_0, before_1),
            "nor paid for — a refusal costs nothing"
        );

        // The same address takes the twig doorway, so the refusal above is
        // about the rung and not about the spot.
        place(
            SEED,
            &bc,
            &nod,
            &mut pieces,
            &mut p,
            0,
            3,
            CX,
            CZ,
            0,
            LOC_EDGE_N,
            &mut ev,
        );
        assert_eq!(last(&ev).0, crate::world::EV_PIECE_PLACED);
        assert_eq!(pieces.len(), before_pieces + 1);
    }

    #[test]
    fn upgrade_refuses_sideways_downward_and_missing_rungs() {
        let (bc, mut pieces, nod, mut ev, mut p) = walled(&[(0, 100), (1, 100)]);
        // Same material, and a material the ladder has no rung for.
        for mat in [MAT_WOOD, MAT_METAL] {
            upgrade(
                &bc,
                &nod,
                &mut pieces,
                &mut p,
                CX,
                CZ,
                0,
                LOC_EDGE_W,
                mat,
                &mut ev,
            );
            assert_eq!(
                last(&ev),
                (crate::world::EV_BUILD_REFUSED, 7, REFUSE_B_TIER)
            );
        }
        // A shape with nothing above twig refuses the same way, and the
        // **doorway** is that shape — twig v0 gave the foundation and the
        // floor stone rungs of their own (rows 5 and 6), so the doorway is
        // the only one left with a ceiling at twig. Put one on the other
        // edge and ask it to climb.
        place(
            SEED,
            &bc,
            &nod,
            &mut pieces,
            &mut p,
            0,
            3,
            CX,
            CZ,
            0,
            LOC_EDGE_N,
            &mut ev,
        );
        assert_eq!(last(&ev).0, crate::world::EV_PIECE_PLACED);
        upgrade(
            &bc,
            &nod,
            &mut pieces,
            &mut p,
            CX,
            CZ,
            0,
            LOC_EDGE_N,
            MAT_STONE,
            &mut ev,
        );
        assert_eq!(last(&ev).2, REFUSE_B_TIER);
        // Downward: climb to stone, then ask for wood back.
        upgrade(
            &bc,
            &nod,
            &mut pieces,
            &mut p,
            CX,
            CZ,
            0,
            LOC_EDGE_W,
            MAT_STONE,
            &mut ev,
        );
        assert_eq!(last(&ev).0, crate::world::EV_PIECE_PLACED);
        upgrade(
            &bc,
            &nod,
            &mut pieces,
            &mut p,
            CX,
            CZ,
            0,
            LOC_EDGE_W,
            MAT_WOOD,
            &mut ev,
        );
        assert_eq!(last(&ev).2, REFUSE_B_TIER);
        assert_eq!(
            pieces.find(CX, CZ, 0, LOC_EDGE_W).unwrap().row,
            4,
            "a refused downgrade left the stone standing"
        );
    }

    #[test]
    fn upgrade_refuses_empty_air_distance_and_an_empty_purse() {
        let (bc, mut pieces, nod, mut ev, mut p) = walled(&[(0, 100), (1, 2)]);
        // Nothing at the north edge.
        upgrade(
            &bc,
            &nod,
            &mut pieces,
            &mut p,
            CX,
            CZ,
            0,
            LOC_EDGE_N,
            MAT_STONE,
            &mut ev,
        );
        assert_eq!(last(&ev).2, REFUSE_B_SPOT);

        // The wall is there, the stone is not (2 units against a cost of 4).
        upgrade(
            &bc,
            &nod,
            &mut pieces,
            &mut p,
            CX,
            CZ,
            0,
            LOC_EDGE_W,
            MAT_STONE,
            &mut ev,
        );
        assert_eq!(last(&ev).2, REFUSE_B_COST);
        assert_eq!(inv_count(&p.inv, 1), 2, "a refusal charged nothing");
        assert_eq!(pieces.find(CX, CZ, 0, LOC_EDGE_W).unwrap().row, 1);

        // Rich, but standing 20 m off.
        let mut far = player_at_cell_center(&[(1, 100)]);
        far.body = Body::at(
            SEED,
            (CX as f32 + 7.5) * BUILD_CELL_M,
            (CZ as f32 + 0.5) * BUILD_CELL_M,
        );
        upgrade(
            &bc,
            &nod,
            &mut pieces,
            &mut far,
            CX,
            CZ,
            0,
            LOC_EDGE_W,
            MAT_STONE,
            &mut ev,
        );
        assert_eq!(last(&ev).2, REFUSE_B_REACH);
        assert_eq!(inv_count(&far.inv, 1), 100, "a refusal charged nothing");
    }

    #[test]
    fn upgrade_never_moves_collision() {
        let (bc, mut pieces, nod, mut ev, mut p) = walled(&[(0, 100), (1, 100)]);
        // A doorway beside the wall, sealed the way a placed door seals it.
        place(
            SEED,
            &bc,
            &nod,
            &mut pieces,
            &mut p,
            0,
            3,
            CX,
            CZ,
            0,
            LOC_EDGE_N,
            &mut ev,
        );
        assert_eq!(last(&ev).0, crate::world::EV_PIECE_PLACED);
        pieces.set_door(CX, CZ, 0, LOC_EDGE_N, true);
        let before = pieces.cols().get(CX, CZ);

        upgrade(
            &bc,
            &nod,
            &mut pieces,
            &mut p,
            CX,
            CZ,
            0,
            LOC_EDGE_W,
            MAT_STONE,
            &mut ev,
        );
        assert_eq!(last(&ev).0, crate::world::EV_PIECE_PLACED);
        assert_eq!(
            pieces.cols().get(CX, CZ),
            before,
            "the shape never changed, so no mask may have"
        );
    }

    #[test]
    fn upgrade_answers_to_a_foreign_claim() {
        let bc = BuildContent::probe_fixture();
        let dc = crate::deploy::DeployContent::probe_fixture();
        let mut pieces = Pieces::new();
        let mut deploys = Deploys::new();
        let mut ev = EventQueue::default();
        let mut owner = player_at_cell_center(&[(0, 100), (1, 100), (2, 100), (3, 100)]);
        for (row, loc) in [(0u16, LOC_PLANE), (1u16, LOC_EDGE_W)] {
            place(
                SEED,
                &bc,
                &deploys,
                &mut pieces,
                &mut owner,
                0,
                row,
                CX,
                CZ,
                0,
                loc,
                &mut ev,
            );
            assert_eq!(last(&ev).0, crate::world::EV_PIECE_PLACED);
        }
        // The fixture's hearth row, planted on the owner's foundation.
        let hearth = (0..dc.def_count)
            .find(|&r| dc.defs[r as usize].arch == crate::deploy::ARCH_HEARTH)
            .expect("fixture holds a hearth");
        crate::deploy::place_deploy(
            SEED,
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut owner,
            0,
            hearth,
            CX,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(last(&ev).0, crate::world::EV_DEPLOY_PLACED, "hearth stands");

        let mut stranger = player_at_cell_center(&[(1, 100)]);
        stranger.id = 9;
        upgrade(
            &bc,
            &deploys,
            &mut pieces,
            &mut stranger,
            CX,
            CZ,
            0,
            LOC_EDGE_W,
            MAT_STONE,
            &mut ev,
        );
        assert_eq!(
            last(&ev),
            (crate::world::EV_BUILD_REFUSED, 9, REFUSE_B_CLAIM)
        );
        assert_eq!(pieces.find(CX, CZ, 0, LOC_EDGE_W).unwrap().row, 1);

        // The hearth's owner may still climb their own wall's ladder.
        upgrade(
            &bc,
            &deploys,
            &mut pieces,
            &mut owner,
            CX,
            CZ,
            0,
            LOC_EDGE_W,
            MAT_STONE,
            &mut ev,
        );
        assert_eq!(last(&ev).0, crate::world::EV_PIECE_PLACED);
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

    // ---- structural collapse -------------------------------------------

    /// One row per shape, at the row index equal to its own shape code, so
    /// a fixture reads `SHAPE_FLOOR` where the store reads `row`. Covers
    /// all six — the probe fixture has no stairs and no roof, and both are
    /// clauses of the support rule this section is about.
    fn collapse_content() -> BuildContent {
        let shapes = [
            SHAPE_FOUNDATION,
            SHAPE_WALL,
            SHAPE_DOORWAY,
            SHAPE_FLOOR,
            SHAPE_STAIRS,
            SHAPE_ROOF,
        ];
        let mut b = BuildContent::EMPTY;
        b.piece_count = shapes.len() as u16;
        for (row, &shape) in shapes.iter().enumerate() {
            b.pieces[row] = PieceDef {
                shape,
                material: MAT_TWIG,
                hp: 100,
                n_costs: 0,
                costs: [(0, 0); MAX_PIECE_COSTS],
            };
        }
        b
    }

    /// Put a piece straight in the store, and only where the same
    /// `supported()` the place verb runs would have taken it: a fixture
    /// that could never have been built is not evidence about a base.
    fn try_put(pieces: &mut Pieces, cx: u16, cz: u16, level: u8, loc: u8, shape: u8) -> bool {
        if !loc_fits_shape(shape, loc)
            || level >= MAX_BUILD_LEVELS as u8
            || pieces.find(cx, cz, level, loc).is_some()
            || !supported(pieces, shape, cx, cz, level, loc)
        {
            return false;
        }
        pieces.insert(
            PieceRec {
                cx,
                cz,
                level,
                loc,
                row: shape,
                facing: 0,
                hp: 100,
                uh: 0,
            },
            shape,
            0,
        )
    }

    /// Grow a base up and out from a single foundation, taking every piece
    /// the rules accept, until `target` stand. Nothing else in it touches
    /// the ground, so what that foundation's removal owes is the whole
    /// structure — and the count is the assertion.
    fn grow_from_one_foundation(pieces: &mut Pieces, cx: u16, cz: u16, target: usize) {
        assert!(try_put(pieces, cx, cz, 0, LOC_PLANE, SHAPE_FOUNDATION));
        let r = MAX_BUILD_LEVELS as u16 + 1; // the diamond spreads one cell a storey
        let top = MAX_BUILD_LEVELS as u8 - 1;
        for level in 0..MAX_BUILD_LEVELS as u8 {
            for pass in 0..2 {
                for dz in 0..=2 * r {
                    for dx in 0..=2 * r {
                        if pieces.len() >= target {
                            return;
                        }
                        let (x, z) = (cx + dx - r, cz + dz - r);
                        if pass == 0 {
                            // Risers and edges on whatever stands here now,
                            // one at a time so `target` lands exactly. The
                            // wall/doorway roles alternate across the grid:
                            // `supported()` treats the two alike but the
                            // column index files them in different masks,
                            // and a fixture that only ever puts doorways on
                            // north edges never probes the west one.
                            let alt = (x as u32 + z as u32 + level as u32).is_multiple_of(2);
                            let (we, ne) = if alt {
                                (SHAPE_WALL, SHAPE_DOORWAY)
                            } else {
                                (SHAPE_DOORWAY, SHAPE_WALL)
                            };
                            for &(loc, shape) in &[
                                (LOC_RISER, SHAPE_STAIRS),
                                (LOC_EDGE_W, we),
                                (LOC_EDGE_N, ne),
                            ] {
                                if pieces.len() >= target {
                                    return;
                                }
                                try_put(pieces, x, z, level, loc, shape);
                            }
                        } else if level < top {
                            // Then the planes those edges now carry.
                            let shape = if level + 1 == top {
                                SHAPE_ROOF
                            } else {
                                SHAPE_FLOOR
                            };
                            try_put(pieces, x, z, level + 1, LOC_PLANE, shape);
                        }
                    }
                }
            }
        }
    }

    /// Four bare stacks of edges rising off one foundation, with no plane
    /// above ground anywhere. The diamond generator cannot produce this —
    /// it takes every floor the rules allow — and without it the fixture
    /// set never contains a piece whose **sole** support is the edge
    /// directly below it, which is one whole clause of `supported()`. A
    /// reverse map that forgets that clause passes every other fixture
    /// here, because a fallen plane happens to re-name the same edges.
    fn grow_wall_stacks(pieces: &mut Pieces) {
        assert!(try_put(pieces, CX, CZ, 0, LOC_PLANE, SHAPE_FOUNDATION));
        assert!(try_put(pieces, CX, CZ, 0, LOC_RISER, SHAPE_STAIRS));
        for level in 0..MAX_BUILD_LEVELS as u8 {
            // Both roles on both edge locs — all four edge masks live.
            assert!(try_put(pieces, CX, CZ, level, LOC_EDGE_W, SHAPE_WALL));
            assert!(try_put(pieces, CX, CZ, level, LOC_EDGE_N, SHAPE_DOORWAY));
            assert!(try_put(
                pieces,
                CX + 1,
                CZ,
                level,
                LOC_EDGE_W,
                SHAPE_DOORWAY
            ));
            assert!(try_put(pieces, CX, CZ + 1, level, LOC_EDGE_N, SHAPE_WALL));
        }
    }

    /// The same stacks with a landing partway up: above it a wall has two
    /// legs (the edge below and the plane beside), below it only one, so
    /// one structure holds both cases and the transition between them.
    fn grow_stacks_and_a_landing(pieces: &mut Pieces) {
        grow_wall_stacks(pieces);
        assert!(try_put(pieces, CX, CZ, 4, LOC_PLANE, SHAPE_FLOOR));
        assert!(try_put(pieces, CX, CZ, 4, LOC_RISER, SHAPE_STAIRS));
        assert!(try_put(pieces, CX, CZ + 1, 5, LOC_PLANE, SHAPE_ROOF));
    }

    /// "Standing" applied the slow honest way: scan every piece, drop any
    /// whose `supported()` is false, repeat until a pass changes nothing.
    /// Support is monotone — removing a piece can only take support away —
    /// so this fixed point is unique whatever order it removes in, which
    /// is what makes it a shadow `collapse_from` can be compared against.
    fn naive_settle(pieces: &mut Pieces, bc: &BuildContent) {
        loop {
            let doomed = (0..pieces.len()).find(|&i| !stands(pieces, bc, i));
            match doomed {
                Some(i) => {
                    let row = pieces.entries()[i].row;
                    pieces.remove_at(i, bc.pieces[row as usize].shape);
                }
                None => return,
            }
        }
    }

    fn addresses(pieces: &Pieces) -> Vec<(u16, u16, u8, u8)> {
        let mut a: Vec<_> = pieces
            .entries()
            .iter()
            .map(|r| (r.cx, r.cz, r.level, r.loc))
            .collect();
        a.sort_unstable();
        a
    }

    /// The gate on `dependents` being the exact inverse of `supported()`.
    /// Take out **every** piece of six real bases in turn and require the
    /// cascade to land on the same base the naive fixed point does — so a
    /// missing clause under-removes and either way this fails. Nothing
    /// else here would catch a reverse map that is merely nearly right.
    ///
    /// What it pins and what it does not, measured by mutation rather than
    /// asserted: dropping any one clause of `dependents`, aiming one at the
    /// wrong cell or level, cutting the propagation off after the first
    /// ring, or losing any mask in `occupied_at` all fail here. A broken
    /// `stands()` does **not** — the shadow calls the same `stands()`, so
    /// both sides degrade together — and that is the division of labour on
    /// purpose: `stands()` is pinned by the three behavioural tests below,
    /// all of which fail when it is broken. An over-broad `dependents` also
    /// survives, and correctly so: a candidate that still stands is skipped,
    /// so a superset costs work and never an outcome.
    #[test]
    fn collapse_matches_a_naive_fixed_point() {
        // (name, builder, the piece count it owes — a fixture that quietly
        // shrinks is a gate that quietly stops covering anything).
        type Fixture = (&'static str, fn(&mut Pieces), usize);
        let fixtures: &[Fixture] = &[
            ("diamond-6", |p| grow_from_one_foundation(p, CX, CZ, 6), 6),
            (
                "diamond-17",
                |p| grow_from_one_foundation(p, CX, CZ, 17),
                17,
            ),
            (
                "diamond-33",
                |p| grow_from_one_foundation(p, CX, CZ, 33),
                33,
            ),
            (
                "diamond-50",
                |p| grow_from_one_foundation(p, CX, CZ, 50),
                50,
            ),
            ("wall-stacks", grow_wall_stacks, 34),
            ("stacks-and-a-landing", grow_stacks_and_a_landing, 37),
        ];
        let bc = collapse_content();
        let dc = DeployContent::EMPTY;
        for &(name, build_fixture, expect) in fixtures {
            let mut reference = Pieces::new();
            build_fixture(&mut reference);
            let n = reference.len();
            assert_eq!(n, expect, "fixture {name} owes exactly {expect} pieces");

            for &(vx, vz, vl, vloc) in addresses(&reference).iter() {
                let mut fast = Pieces::new();
                let mut slow = Pieces::new();
                build_fixture(&mut fast);
                build_fixture(&mut slow);
                let mut nod = Deploys::new();
                let mut ev = EventQueue::default();

                let shape = {
                    let row = fast.find(vx, vz, vl, vloc).unwrap().row;
                    bc.pieces[row as usize].shape
                };
                let i = fast.find_index(vx, vz, vl, vloc).unwrap();
                fast.remove_at(i, shape);
                let fell = collapse_from(
                    &dc,
                    &bc,
                    &mut fast,
                    &mut nod,
                    (vx, vz, vl, vloc),
                    &mut tick_budget(),
                    &mut ev,
                );

                let j = slow.find_index(vx, vz, vl, vloc).unwrap();
                slow.remove_at(j, shape);
                naive_settle(&mut slow, &bc);

                let victim = (vx, vz, vl, vloc);
                assert!(
                    fell < MAX_COLLAPSE_PIECES,
                    "fixture {name} reached the cap on {victim:?} — the \
                     comparison below only means anything under it"
                );
                assert_eq!(
                    addresses(&fast),
                    addresses(&slow),
                    "removing {victim:?} from fixture {name} left a different \
                     world than the fixed point"
                );
                assert_eq!(
                    fell,
                    n - 1 - fast.len(),
                    "the returned count disagrees with the store it left"
                );
            }
        }
    }

    /// The gap this exists for: take the legs out and the base comes down.
    #[test]
    fn a_foundation_takes_its_whole_tower_with_it() {
        let bc = collapse_content();
        let dc = DeployContent::EMPTY;
        let mut pieces = Pieces::new();
        let mut nod = Deploys::new();
        let mut ev = EventQueue::default();
        grow_from_one_foundation(&mut pieces, CX, CZ, 40);
        let n = pieces.len();
        let before: Vec<PieceRec> = pieces.entries().to_vec();

        let i = pieces.find_index(CX, CZ, 0, LOC_PLANE).unwrap();
        crate::deploy::drop_piece(&dc, &mut pieces, &mut nod, i, SHAPE_FOUNDATION, &mut ev);
        let fell = collapse_from(
            &dc,
            &bc,
            &mut pieces,
            &mut nod,
            (CX, CZ, 0, LOC_PLANE),
            &mut tick_budget(),
            &mut ev,
        );

        assert_eq!(
            pieces.len(),
            0,
            "{} pieces left hanging in the air",
            pieces.len()
        );
        assert_eq!(fell, n - 1);
        // The derived collision view came down with it — a wall that is
        // gone from the store and still in the column index is a wall the
        // player walks into (collide.rs).
        assert!(pieces.cols().is_empty());
        // And every removal announced itself: one the client never hears
        // is a piece drawn for the rest of the session.
        assert_eq!(
            ev.entries()
                .iter()
                .filter(|e| e.code == crate::world::EV_PIECE_REMOVED)
                .count(),
            n
        );
        assert_eq!(ev.dropped, 0, "the collapse overflowed the event ring");

        // Counting the removals says they happened; it does not say they
        // named the right pieces. `EV_PIECE_REMOVED`'s payload is law with
        // no gate (CLAUDE.md's positional trap): swap `a` and `b` at the
        // emit site, or shift `level`/`loc`/`row` inside the packed `b`,
        // and the encoder is untouched (`test_protocol_golden` green), the
        // ring is not in `state_hash` (`test_replay` green), and every
        // field is a `u32` (clippy green). This collapse is a third
        // producer of that code and this is the pin: the multiset of
        // (a, b) the tick announced against the addresses the fixture
        // actually stood, built here from the store snapshot rather than
        // from anything the emit site said.
        //
        // The fixture is what makes it sharp — it spans many cells, all
        // eight levels, all four locs and five rows, so no two subfields
        // hold the same value everywhere and a shift between them cannot
        // survive the compare.
        let mut expect: Vec<(u32, u32)> = before
            .iter()
            .map(|r| {
                (
                    crate::gather::cell_key(r.cx, r.cz),
                    ((r.level as u32) << 16) | ((r.loc as u32) << 8) | r.row as u32,
                )
            })
            .collect();
        let mut got: Vec<(u32, u32)> = ev
            .entries()
            .iter()
            .filter(|e| e.code == crate::world::EV_PIECE_REMOVED)
            .map(|e| (e.a, e.b))
            .collect();
        expect.sort_unstable();
        got.sort_unstable();
        assert_eq!(got, expect, "a removal named the wrong piece");
    }

    /// The other half, and the one an over-eager cascade fails: a piece
    /// with a second leg keeps standing, and stairs are load-bearing for
    /// nothing.
    #[test]
    fn a_second_leg_holds_what_the_first_one_dropped() {
        let bc = collapse_content();
        let dc = DeployContent::EMPTY;
        let mut pieces = Pieces::new();
        let mut nod = Deploys::new();
        let mut ev = EventQueue::default();

        assert!(try_put(&mut pieces, CX, CZ, 0, LOC_PLANE, SHAPE_FOUNDATION));
        assert!(try_put(
            &mut pieces,
            CX + 1,
            CZ,
            0,
            LOC_PLANE,
            SHAPE_FOUNDATION
        ));
        // Adjoins only the foundation that is about to go.
        assert!(try_put(&mut pieces, CX, CZ, 0, LOC_EDGE_W, SHAPE_WALL));
        // The shared boundary: adjoins both cells, so it keeps a leg.
        assert!(try_put(&mut pieces, CX + 1, CZ, 0, LOC_EDGE_W, SHAPE_WALL));
        assert!(try_put(&mut pieces, CX + 1, CZ, 0, LOC_RISER, SHAPE_STAIRS));
        assert!(try_put(&mut pieces, CX + 1, CZ, 1, LOC_PLANE, SHAPE_FLOOR));

        let i = pieces.find_index(CX, CZ, 0, LOC_PLANE).unwrap();
        crate::deploy::drop_piece(&dc, &mut pieces, &mut nod, i, SHAPE_FOUNDATION, &mut ev);
        let fell = collapse_from(
            &dc,
            &bc,
            &mut pieces,
            &mut nod,
            (CX, CZ, 0, LOC_PLANE),
            &mut tick_budget(),
            &mut ev,
        );

        assert_eq!(fell, 1, "only the wall standing on one leg should fall");
        assert!(pieces.find(CX, CZ, 0, LOC_EDGE_W).is_none());
        assert!(pieces.find(CX + 1, CZ, 0, LOC_EDGE_W).is_some());
        assert!(pieces.find(CX + 1, CZ, 1, LOC_PLANE).is_some());
        assert!(pieces.find(CX + 1, CZ, 0, LOC_RISER).is_some());
    }

    /// `MAX_COLLAPSE_PIECES` is a cap on one tick, not a hole: what it
    /// defers, `support_sweep` finishes. Both halves asserted here,
    /// because either alone would let the other rot.
    #[test]
    fn the_cap_defers_the_rest_and_the_sweep_finishes_it() {
        let bc = collapse_content();
        let dc = DeployContent::EMPTY;
        let mut pieces = Pieces::new();
        let mut nod = Deploys::new();
        let mut ev = EventQueue::default();
        grow_from_one_foundation(&mut pieces, CX, CZ, MAX_COLLAPSE_PIECES * 2);
        let n = pieces.len();
        assert!(
            n > MAX_COLLAPSE_PIECES + 1,
            "the fixture must outgrow the cap"
        );

        let i = pieces.find_index(CX, CZ, 0, LOC_PLANE).unwrap();
        crate::deploy::drop_piece(&dc, &mut pieces, &mut nod, i, SHAPE_FOUNDATION, &mut ev);
        let fell = collapse_from(
            &dc,
            &bc,
            &mut pieces,
            &mut nod,
            (CX, CZ, 0, LOC_PLANE),
            &mut tick_budget(),
            &mut ev,
        );

        // Stopped at the cap, and stopped honestly — the rest is still
        // there, not silently dropped and not silently forgotten.
        assert_eq!(fell, MAX_COLLAPSE_PIECES);
        assert_eq!(pieces.len(), n - 1 - MAX_COLLAPSE_PIECES);

        let mut cursor = 0u32;
        let mut ticks = 0usize;
        while !pieces.is_empty() {
            ticks += 1;
            assert!(
                ticks < 4096,
                "support_sweep never finished: {} pieces still in the air",
                pieces.len()
            );
            ev.clear();
            support_sweep(
                &dc,
                &bc,
                &mut pieces,
                &mut nod,
                &mut cursor,
                &mut tick_budget(),
                &mut ev,
            );
            assert_eq!(ev.dropped, 0, "a sweep tick overflowed the event ring");
        }
    }

    /// The cap that composes, driven through `World::tick` — the only
    /// level it can be checked at, because the hole was never inside one
    /// cascade. `MAX_COLLAPSE_PIECES` bounds a cascade and a tick holds
    /// many: `upkeep_sweep` does not stop at its first removal the way
    /// `support_sweep` does, so its 64 visits can each seed one. Every
    /// unit test above passes a fresh budget and would pass unchanged with
    /// no per-tick bound at all; this is the one that would not.
    ///
    /// Armed rather than waited for: every piece is set to 1 hp and an
    /// unpaid upkeep hour, so the sweep's next visit kills it outright and
    /// the whole wave lands in one tick instead of over three real decay
    /// periods. No hearth stands, so nothing pays. Nobody has joined, so
    /// no other producer is competing for the ring — a `dropped` here is
    /// this path's and nothing else's.
    #[test]
    fn a_tick_that_collapses_many_bases_at_once_drops_no_removal() {
        const TOWERS: u16 = 8;
        const SPACING: u16 = 32; // wider than the diamond a tower spreads

        let mut w = crate::world::World::new(SEED);
        w.build = collapse_content();
        // `mat_count` non-zero is what arms decay at all (`upkeep_sweep`
        // returns early on an inert table).
        w.deploy = DeployContent::probe_fixture();
        // `grow_from_one_foundation`'s target is the whole store, not this
        // tower — so it climbs.
        for k in 0..TOWERS {
            grow_from_one_foundation(&mut w.pieces, CX + k * SPACING, CZ, 40 * (k as usize + 1));
        }
        let standing = w.pieces.len();
        assert!(
            standing > MAX_REMOVALS_PER_TICK * 2,
            "the fixture must outrun one tick's budget: {standing} pieces"
        );
        for i in 0..standing {
            w.pieces.set_upkeep(i, 1, 0);
        }
        w.tick = UPKEEP_PERIOD_TICKS; // one upkeep hour due on everything

        let mut removed = 0usize;
        let mut ticks = 0usize;
        let mut worst = 0usize;
        while !w.pieces.is_empty() {
            ticks += 1;
            assert!(ticks < 4096, "{} pieces never came down", w.pieces.len());
            let before = w.pieces.len();
            w.tick(&[]);
            let fell = before - w.pieces.len();
            // The assertion the whole slice exists for. A dropped removal
            // is the one event whose loss is permanent: the piece is gone
            // from the store and drawn on every screen for the rest of the
            // session, and no later tick re-announces it.
            assert_eq!(
                w.events.dropped, 0,
                "tick {ticks} overflowed the event ring: {fell} pieces fell"
            );
            assert!(
                fell <= MAX_REMOVALS_PER_TICK,
                "tick {ticks} removed {fell} pieces, over the budget"
            );
            worst = worst.max(fell);
            removed += fell;
        }

        // Deferred, never lost: the budget costs ticks and not pieces.
        assert_eq!(removed, standing, "the defer policy lost a piece");
        assert!(w.pieces.cols().is_empty(), "the collision view outlived it");
        // And it really did press against the budget — a fixture that
        // never reached it would assert nothing.
        assert_eq!(
            worst, MAX_REMOVALS_PER_TICK,
            "the fixture never spent a whole tick's budget, so the bound was untested"
        );
        assert!(
            ticks > 1,
            "one tick took the lot, so nothing was ever deferred"
        );
    }

    /// **A box may not stand at the one address that packs to handle 0.**
    /// The minting half of the pair `deploy::box_index` guards on decode;
    /// `tests/box_container.rs` owns the decode half.
    ///
    /// `box_key(0, 0, 0)` is 0, and 0 is the reserved "no container open"
    /// handle on every layer that carries one. A box there could be opened
    /// by nobody, so the address is refused as a spot rather than left to
    /// swallow a player's items — the same posture `Backpacks` takes by
    /// starting its ids at 1.
    ///
    /// It lives here rather than beside the other box tests because the
    /// address is at the world's origin corner, which is water under every
    /// seed: `Command::Place` cannot stand the foundation the box needs,
    /// and a test that let terrain do the refusing would assert nothing.
    /// `try_put` writes the piece straight in, so what refuses below is
    /// the address arithmetic and only that.
    #[test]
    fn a_box_is_refused_at_the_one_address_that_packs_to_zero() {
        use crate::deploy::{box_key, place_deploy, DeployDef, ARCH_BOX, ARCH_HEARTH};
        use crate::world::EV_DEPLOY_REFUSED;

        assert_eq!(box_key(0, 0, 0), 0, "the corner this guards has moved");

        let mut dc = DeployContent::EMPTY;
        dc.def_count = 2;
        dc.defs[0] = DeployDef {
            arch: ARCH_BOX,
            placement: crate::deploy::PLACE_FOUNDATION,
            hp: 100,
            item: 0,
            ..DeployDef::INERT
        };
        // A second, non-box deployable on the identical footing: the
        // refusal must be about the *handle*, not about the cell, and
        // nothing without a container handle has anything to lose here.
        dc.defs[1] = DeployDef {
            arch: ARCH_HEARTH,
            placement: crate::deploy::PLACE_FOUNDATION,
            hp: 100,
            item: 0,
            ..DeployDef::INERT
        };
        let bc = collapse_content();
        let mut pieces = Pieces::new();
        let mut deploys = Deploys::new();
        let mut ev = EventQueue::default();

        assert!(try_put(&mut pieces, 0, 0, 0, LOC_PLANE, SHAPE_FOUNDATION));

        let mut p = Player {
            id: 7,
            active: true,
            body: Body::at(SEED, 0.5 * BUILD_CELL_M, 0.5 * BUILD_CELL_M),
            ..Player::default()
        };
        p.inv[0] = ItemStack { item: 0, count: 9 };

        place_deploy(
            SEED,
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut p,
            0,
            0,
            0,
            0,
            0,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(deploys.len(), 0, "a box took the handle-0 address");
        assert_eq!(
            last(&ev),
            (EV_DEPLOY_REFUSED, p.id, crate::deploy::REFUSE_D_SPOT),
            "refused, but not as a spot — the placer has to hear why"
        );

        // The same cell, the same footing, a deployable with no container
        // handle: allowed. The refusal is one address of one arch, not a
        // dead cell.
        ev.clear();
        place_deploy(
            SEED,
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut p,
            0,
            1,
            0,
            0,
            0,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(deploys.len(), 1, "a hearth was refused the same cell");

        // And a box one storey up is fine, because that address is not 0.
        assert_ne!(box_key(0, 0, 1), 0);
    }

    /// The other half of the composition, and the one with no bound of its
    /// own at all: the decay sweep is at least held to
    /// `UPKEEP_SWEEP_PER_TICK` visits, but nothing limits how many raiders
    /// land a killing blow in the same tick. `MAX_PLAYERS` is 100 and each
    /// swing can seed a cascade of `MAX_COLLAPSE_PIECES`, so the arithmetic
    /// runs to thousands of removals against a 256-slot ring.
    ///
    /// One budget, many swings — the same `&mut usize` `World::tick` hands
    /// every path. The assertion is that the budget binds *across* calls
    /// and that running out is a deferral and not a loss: the wall that
    /// could not be removed is still standing, still in the collision
    /// view, and still at hp the next swing can take off.
    #[test]
    fn many_raiders_in_one_tick_share_one_budget_and_the_last_wall_survives() {
        let bc = collapse_content();
        let dc = DeployContent::EMPTY;
        let mut pieces = Pieces::new();
        let mut nod = Deploys::new();
        let mut ev = EventQueue::default();

        // Free-standing foundations: no cascades, so the count is exactly
        // the number of killing blows and the budget is the only thing
        // that can stop them.
        let walls = MAX_REMOVALS_PER_TICK + 8;
        for k in 0..walls {
            assert!(try_put(
                &mut pieces,
                CX + (k as u16) * 4,
                CZ,
                0,
                LOC_PLANE,
                SHAPE_FOUNDATION,
            ));
        }
        assert_eq!(pieces.len(), walls);

        let mut budget = MAX_REMOVALS_PER_TICK;
        let mut fell = 0usize;
        for k in 0..walls {
            let i = pieces
                .find_index(CX + (k as u16) * 4, CZ, 0, LOC_PLANE)
                .unwrap();
            if crate::deploy::damage_piece(
                &dc,
                &bc,
                &mut pieces,
                &mut nod,
                i,
                100,
                &mut budget,
                &mut ev,
            ) {
                fell += 1;
            }
        }

        assert_eq!(fell, MAX_REMOVALS_PER_TICK, "the budget did not bind");
        assert_eq!(budget, 0);
        assert_eq!(ev.dropped, 0, "the tick overflowed the event ring");

        // Deferred, not lost. The eight that outlived the budget are still
        // standing, still solid, and still killable — each stopped one hp
        // short rather than being parked at zero, which is the state no
        // sweep would ever clear.
        assert_eq!(pieces.len(), walls - MAX_REMOVALS_PER_TICK);
        for r in pieces.entries() {
            assert_eq!(r.hp, 1, "a deferred piece was left at a resting hp");
        }
        assert!(!pieces.cols().is_empty(), "the collision view lost them");

        // And the next tick's budget takes them, which is what makes the
        // deferral cost latency rather than correctness.
        let mut next = MAX_REMOVALS_PER_TICK;
        ev.clear();
        while !pieces.is_empty() {
            crate::deploy::damage_piece(
                &dc,
                &bc,
                &mut pieces,
                &mut nod,
                0,
                100,
                &mut next,
                &mut ev,
            );
        }
        assert_eq!(ev.dropped, 0);
    }

    /// The wiring, not the function: `damage_piece` is what a raid swing
    /// reaches (combat.rs picks the target), so this is the path a raider
    /// actually walks — kill the wall at the bottom and the storeys it
    /// carried come with it, while the foundation stands.
    #[test]
    fn a_raid_swing_that_kills_a_wall_brings_down_what_it_held() {
        let bc = collapse_content();
        let dc = DeployContent::EMPTY;
        let mut pieces = Pieces::new();
        let mut nod = Deploys::new();
        let mut ev = EventQueue::default();

        assert!(try_put(&mut pieces, CX, CZ, 0, LOC_PLANE, SHAPE_FOUNDATION));
        assert!(try_put(&mut pieces, CX, CZ, 0, LOC_EDGE_W, SHAPE_WALL));
        assert!(try_put(&mut pieces, CX, CZ, 1, LOC_PLANE, SHAPE_FLOOR));
        assert!(try_put(&mut pieces, CX, CZ, 1, LOC_EDGE_W, SHAPE_WALL));
        assert!(try_put(&mut pieces, CX, CZ, 2, LOC_PLANE, SHAPE_FLOOR));
        assert_eq!(pieces.len(), 5);

        let i = pieces.find_index(CX, CZ, 0, LOC_EDGE_W).unwrap();
        let died = crate::deploy::damage_piece(
            &dc,
            &bc,
            &mut pieces,
            &mut nod,
            i,
            100,
            &mut tick_budget(),
            &mut ev,
        );

        assert!(died);
        assert_eq!(addresses(&pieces), vec![(CX, CZ, 0, LOC_PLANE)]);
        assert_eq!(
            ev.entries()
                .iter()
                .filter(|e| e.code == crate::world::EV_PIECE_REMOVED)
                .count(),
            4,
            "the swung-at wall and the three storeys it carried"
        );
        assert_eq!(
            ev.entries()
                .iter()
                .filter(|e| e.code == crate::world::EV_STRUCT_HIT)
                .count(),
            1,
            "the swing is one hit; the collapse is not four more"
        );
    }
}
