//! Deployables + the hearth (DESIGN.md §2, M1): placeable entities baked
//! from `content/deployables.toml`, the hearth's building-privilege claim,
//! and the upkeep/decay sweep that makes the map heal itself. Pure and
//! fixed-capacity like the other verbs: content reaches the sim only as
//! the baked `DeployContent` table, the inert `EMPTY` default makes every
//! request refuse, and `probe_fixture()` is the synthetic table for the
//! parity/replay/alloc gates.
//!
//! Model v0 (proposed defaults, DECISIONS.md §open "deployables v0" and
//! "upkeep/decay v0"):
//! - A deployable occupies one grid address — body deploys at
//!   (cell, level, LOC_PLANE), doors at a doorway's edge address. Placing
//!   consumes one unit of the deployable's own item.
//! - Placement class (schema order): `ground` = level 0 on buildable
//!   terrain with no plane piece; `foundation` = on a plane piece at the
//!   level; `doorway` = in a doorway edge piece; `any` = foundation, else
//!   ground.
//! - A **hearth** claims building privilege in `HEARTH_RADIUS_M`: piece
//!   and deployable placement inside a *foreign* hearth's radius refuses
//!   (`REFUSE_B_CLAIM` / `REFUSE_D_CLAIM`). No hearth may be placed
//!   inside any hearth's radius, own included.
//! - **Upkeep**: every `UPKEEP_PERIOD_TICKS`, each placed piece charges
//!   `ceil(cost × upkeep_pct_per_day / 100 / 24)` per cost row from the
//!   first hearth (list order) in radius that can pay the whole charge.
//!   Unpaid pieces **decay** `DECAY_PCT_PER_PERIOD`% of max hp (min 1)
//!   per period; at 0 hp the piece is removed, and any deployable at the
//!   removed piece's exact address goes with it.
//! - Deployables charge nothing: covered by any hearth with stock, they
//!   are free; uncovered, they decay on the same cadence (the map sheds
//!   its litter). An empty hearth does not cover itself.
//! - The **feed** action moves up to `FEED_CHUNK` units per upkeep
//!   material per press from the feeder's inventory into a hearth in
//!   reach (any hearth — feeding is a gift, only placement is owner-
//!   gated), capped at `STOCK_MAX` per material. Withdrawal waits for the
//!   container UI; stock readback rides the feed ack only.
//!
//! - **Doors** (door v0, DECISIONS.md §open): a door places **closed**
//!   and seals its doorway (collide.rs shut bits — the store keeps them
//!   in lockstep exactly like the piece store keeps its masks). The
//!   **use** action toggles it open/closed for any player within build
//!   reach — subject to the lock below. EV_DOOR announces the new state
//!   for the wire; removal (decay, or the doorway decaying under it)
//!   clears the shut bit with the record. The state is absolute on the
//!   wire, never a delta, so the client that toggled optimistically
//!   (NETCODE.md §6.1) is confirmed or corrected by the same event.
//! - **Locks** (lock v0, DECISIONS.md §open): a door places **locked to
//!   its placer**, and a locked door only uses for its owner — every
//!   other hand bounces with `REFUSE_D_OWNER`. The **lock** action sets
//!   that bit absolutely (not a toggle: two presses racing must not
//!   fight) and only the owner may, so unlocking is how a door becomes
//!   public — a shop front, a shared hut. Locking never moves the leaf:
//!   an open door locks open, and its owner is the one who can shut it.
//!   The bit rides the same EV_DOOR announcement as the open bit, so one
//!   lane carries the whole door state, absolute, to everyone.
//!
//! Not in this slice (documented, not forgotten): no support cascade when
//! a piece decays away (floating pieces keep decaying on their own if
//! unpaid), no *shared* access — a lock answers to one owner id, and
//! codes/crew lists are the question §open asks (the hearth's claim is
//! owner-only for the same reason), no owner on the wire (a stranger's
//! locked door refuses after the press, which the mispredict path already
//! rolls back — DESIGN.md §5.6's own example), no lock item (the lock is a
//! property of the door, not a deployable).

use crate::build::{
    BuildContent, Pieces, LEVEL_H_M, LOC_EDGE_N, LOC_EDGE_W, LOC_PLANE, SHAPE_DOORWAY,
};
use crate::craft::{inv_count, inv_take};
use crate::gather::ItemStack;
use crate::limits::{
    BOX_SLOTS, HEARTH_STOCK_ROWS, MAX_BOXES, MAX_BOX_SPILL_PER_TICK, MAX_BUILD_COORD,
    MAX_BUILD_LEVELS, MAX_DEPLOYS, MAX_DEPLOY_DEFS, MAX_HEARTHS, UPKEEP_SWEEP_PER_TICK,
};
use crate::terrain;
use crate::world::{
    EventQueue, Player, EV_DEPLOY_PLACED, EV_DEPLOY_REFUSED, EV_DEPLOY_REMOVED, EV_DOOR,
    EV_PIECE_REMOVED, EV_STOCK, EV_STRUCT_HIT, STRUCT_DEPLOY_BIT,
};

/// Archetype codes (schema order: CONTENT.md §1 deployable).
pub const ARCH_BAG: u8 = 0;
pub const ARCH_HEARTH: u8 = 1;
pub const ARCH_BOX: u8 = 2;
pub const ARCH_FIRE: u8 = 3;
pub const ARCH_FURNACE: u8 = 4;
pub const ARCH_WORKBENCH: u8 = 5;
pub const ARCH_DOOR: u8 = 6;

/// Placement-class codes (schema order).
pub const PLACE_GROUND: u8 = 0;
pub const PLACE_FOUNDATION: u8 = 1;
pub const PLACE_DOORWAY: u8 = 2;
pub const PLACE_ANY: u8 = 3;

/// Integer refusal reasons (CLAUDE.md wall 3), carried by
/// EV_DEPLOY_REFUSED / the deploy-refused wire subtype.
pub const REFUSE_D_KIND: u32 = 0;
pub const REFUSE_D_SPOT: u32 = 1;
pub const REFUSE_D_SUPPORT: u32 = 2;
pub const REFUSE_D_TERRAIN: u32 = 3;
pub const REFUSE_D_REACH: u32 = 4;
pub const REFUSE_D_COST: u32 = 5;
pub const REFUSE_D_FULL: u32 = 6;
pub const REFUSE_D_CLAIM: u32 = 7;
pub const REFUSE_D_OVERLAP: u32 = 8;
pub const REFUSE_D_BAG_CAP: u32 = 9;
pub const REFUSE_D_HEARTH: u32 = 10;
/// A use request named an address holding no door.
pub const REFUSE_D_DOOR: u32 = 11;
/// The door isn't yours: a use on someone else's **locked** door, or a
/// lock request on a door you didn't place (lock v0).
pub const REFUSE_D_OWNER: u32 = 12;

/// Hearth privilege radius in meters, planar from the hearth's cell
/// center. Proposed default, DECISIONS.md §open ("deployables v0").
pub const HEARTH_RADIUS_M: f32 = 24.0;
/// Upkeep/decay cadence: one period per real hour at the 30 Hz tick.
/// Proposed default, DECISIONS.md §open ("upkeep/decay v0").
pub const UPKEEP_PERIOD_TICKS: u64 = 108_000;
/// Periods per day — the divisor that spreads `upkeep_pct_per_day`
/// (content/balance.toml) over hourly charges. Calendar arithmetic, not
/// a knob.
pub const PERIODS_PER_DAY: u32 = 24;
/// Unpaid decay per period, % of max hp (min 1 hp). Proposed default,
/// DECISIONS.md §open ("upkeep/decay v0").
pub const DECAY_PCT_PER_PERIOD: u32 = 5;
/// Units per upkeep material one feed press moves. Proposed default,
/// DECISIONS.md §open ("deployables v0").
pub const FEED_CHUNK: u32 = 100;
/// Stock ceiling per material per hearth. Proposed default, DECISIONS.md
/// §open ("deployables v0").
pub const STOCK_MAX: u32 = 2_000;
/// Bags one player may have placed (ALPHA.md §1 knob, DECISIONS.md §open
/// "bag cooldown · cap": 8).
pub const BAG_CAP: usize = 8;
/// Ticks a bag sleeps for after a body wakes on it — the other half of the
/// same spoken knob (ALPHA.md §1, DECISIONS.md §open "bag cooldown · cap":
/// **5 min**), in the only unit the sim owns. 5 × 60 × `TICK_HZ` = 9 000 at
/// the 30 Hz tick, written as the literal it evaluates to because the knob
/// registry gate pins declarations, not expressions;
/// `the_cooldown_is_five_minutes_of_ticks` is what holds the arithmetic.
pub const BAG_COOLDOWN_TICKS: u64 = 9_000;
/// Hour-steps one sweep visit processes at most (bounded catch-up; the
/// sweep revisits every entry far inside one period, so >1 only happens
/// after a tick-jump). Bounded-work constant, not a knob.
const SWEEP_CATCHUP_MAX: u32 = 4;

/// One baked deployable row. `hp == 0` ⇒ inert (the empty-table row).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeployDef {
    pub arch: u8,
    pub placement: u8,
    pub hp: u16,
    /// Item index placing consumes one unit of (the deployable's item).
    pub item: u16,
}

impl DeployDef {
    pub const INERT: Self = Self {
        arch: ARCH_BAG,
        placement: PLACE_GROUND,
        hp: 0,
        item: 0,
    };
}

/// The whole deployable ruleset the sim knows, plus the upkeep globals
/// that price decay (baked from `content/balance.toml` §globals and the
/// build table's cost items). Construction input like the gather table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeployContent {
    pub defs: [DeployDef; MAX_DEPLOY_DEFS],
    pub def_count: u16,
    /// Distinct upkeep materials (item indices, ascending) — the union of
    /// the build table's cost items. Hearth stock rows align to these.
    pub mats: [u16; HEARTH_STOCK_ROWS],
    pub mat_count: u8,
    /// `content/balance.toml` globals.upkeep_pct_per_day.
    pub upkeep_pct_per_day: u16,
}

impl DeployContent {
    /// Inert: no deployable exists, every request refuses, nothing
    /// decays (no materials priced). `World::new` starts here.
    pub const EMPTY: Self = Self {
        defs: [DeployDef::INERT; MAX_DEPLOY_DEFS],
        def_count: 0,
        mats: [0; HEARTH_STOCK_ROWS],
        mat_count: 0,
        upkeep_pct_per_day: 0,
    };

    /// Synthetic table for the parity/replay/alloc gates, over the gather
    /// probe fixture's items (fixture, not game content). Rows cover the
    /// hearth, a station, a door, and a ground deploy so every placement
    /// class and the claim/upkeep paths ride the gates.
    pub fn probe_fixture() -> Self {
        let mut d = Self::EMPTY;
        d.def_count = 4;
        d.defs[0] = DeployDef {
            arch: ARCH_HEARTH,
            placement: PLACE_FOUNDATION,
            hp: 100,
            item: 2,
        };
        d.defs[1] = DeployDef {
            arch: ARCH_WORKBENCH,
            placement: PLACE_ANY,
            hp: 80,
            item: 3,
        };
        d.defs[2] = DeployDef {
            arch: ARCH_DOOR,
            placement: PLACE_DOORWAY,
            hp: 60,
            item: 4,
        };
        d.defs[3] = DeployDef {
            arch: ARCH_BAG,
            placement: PLACE_GROUND,
            hp: 50,
            item: 5,
        };
        // The build probe fixture costs items 0 and 1.
        d.mats = [0, 1, 0, 0];
        d.mat_count = 2;
        d.upkeep_pct_per_day = 10;
        d
    }
}

/// One placed deployable. Grid-addressed like a piece; `owner` gates
/// hearth privilege (and the later respawn-on-bag lane), never the wire.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DeployRec {
    pub cx: u16,
    pub cz: u16,
    pub level: u8,
    pub loc: u8,
    /// Baked deployable row this address holds.
    pub row: u8,
    pub owner: u32,
    /// Current hp (decay drains it; damage lands in M2).
    pub hp: u16,
    /// Last upkeep period processed (`tick / UPKEEP_PERIOD_TICKS`).
    pub uh: u16,
    /// Door state (ARCH_DOOR only; false for everything else). Doors
    /// place closed; the use action toggles. Sim state, hashed, and on
    /// the wire (the deploy record's open bit, wire v6).
    pub open: bool,
    /// Lock state (ARCH_DOOR only; false for everything else). Doors
    /// place **locked** to `owner`, and a locked door only uses for its
    /// owner; the lock action sets this absolutely. Sim state, hashed,
    /// and on the wire (the deploy record's locked bit, wire v8).
    pub locked: bool,
}

/// One hearth's claim + stock, in the dense hearth list. Identity is the
/// grid address of its deploy record; stock rows align to
/// `DeployContent::mats`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HearthRec {
    pub cx: u16,
    pub cz: u16,
    pub level: u8,
    pub owner: u32,
    pub stock: [u32; HEARTH_STOCK_ROWS],
}

/// One deployed box's contents, in the dense box list. Identity is the
/// grid address of its deploy record — `(cx, cz, level)`, the same triple
/// a hearth uses — because a box is furniture and has no id to hand out.
/// `loc` is deliberately absent: two boxes never share a cell and a level
/// (placement's occupancy check forbids it), so the triple is already
/// unique, and leaving `loc` out is what lets the client name a box from
/// the deploy record it already drew.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BoxRec {
    pub cx: u16,
    pub cz: u16,
    pub level: u8,
    /// Who placed it. Sim-side only, and — unlike a door — **not** an
    /// access check: see `inventory::CONT_BOX`. Kept because the spill a
    /// broken box leaves has to belong to someone, the way a corpse's bag
    /// does.
    pub owner: u32,
    pub items: [ItemStack; BOX_SLOTS],
}

impl Default for BoxRec {
    fn default() -> Self {
        Self {
            cx: 0,
            cz: 0,
            level: 0,
            owner: 0,
            items: [ItemStack::default(); BOX_SLOTS],
        }
    }
}

impl BoxRec {
    pub fn is_empty(&self) -> bool {
        self.items.iter().all(|s| s.count == 0)
    }
}

/// Pack a box's grid address into the one container handle the move
/// command carries. `cx`/`cz` are bounded by `MAX_BUILD_COORD` (1024, so
/// ten bits each) and `level` by `MAX_BUILD_LEVELS` (8, three bits), which
/// is 23 bits inside a `u32` with room to spare.
///
/// Deliberately **not** `gather::cell_key`: that one packs `cx << 16 | cz`
/// and spends all 32 bits on the pair, leaving nowhere for the level. A
/// box on a second storey is a box, so the level is part of the address.
pub fn box_key(cx: u16, cz: u16, level: u8) -> u32 {
    ((cx as u32) << 16) | ((cz as u32) << 4) | (level as u32 & 0xF)
}

/// The contents of every standing box, plus the spill buffer a removal
/// hands to `world.rs`. Boxed inside `Deploys` — `MAX_BOXES * BOX_SLOTS`
/// stacks is 12 kB of fixed capacity and `World` is built on the stack
/// (`ShardCore::new`, every wire test) where it is already tight, so this
/// takes the same one-allocation-at-construction posture `Backpacks`
/// takes for exactly the same reason. Nothing here allocates in the tick
/// (wall 2).
pub struct BoxStore {
    entries: [BoxRec; MAX_BOXES],
    len: usize,
    /// Contents of boxes removed this tick, awaiting a ground bag.
    /// `drop_deploy` is reached from decay and from a raid and holds
    /// neither the bag store nor the clock, so it parks the record here
    /// and `World::step` stands the bag up before the tick ends.
    spill: [BoxRec; MAX_BOX_SPILL_PER_TICK],
    spill_len: usize,
}

impl BoxStore {
    fn new() -> Self {
        Self {
            entries: [BoxRec::default(); MAX_BOXES],
            len: 0,
            spill: [BoxRec::default(); MAX_BOX_SPILL_PER_TICK],
            spill_len: 0,
        }
    }
}

/// The placed-deployable store: dense, insertion-ordered, plus the dense
/// hearth list the claim checks and the sweep scan, and the dense box list
/// the move verb resolves against. Removal swap-removes (decay order is
/// the sweep cursor's, deterministic); the wire layer restarts in-progress
/// sync walks on any removal.
pub struct Deploys {
    entries: [DeployRec; MAX_DEPLOYS],
    len: usize,
    /// Bag respawn cooldowns, index-aligned to `entries`: the first tick
    /// `entries[i]` may be woken on again. Meaningless for every archetype
    /// that is not `ARCH_BAG`, and zero for a fresh one — `0` is "ready from
    /// the first tick", not a sentinel.
    ///
    /// A parallel array rather than a field on `DeployRec`, and that is a
    /// wire decision rather than a layout preference: `DeployRec` is the
    /// struct the deploy-sync packet mirrors, so eight bytes of sim-only
    /// timer on it would ride 24-deep in `EventMsg::DeploySync` and grow the
    /// client's event enum by 192 bytes it can never read. The client draws
    /// a bag; it does not adjudicate one. Kept aligned by `insert` and by
    /// `remove_at`, whose swap-remove moves both halves together.
    bag_ready: [u64; MAX_DEPLOYS],
    hearths: [HearthRec; MAX_HEARTHS],
    hearth_count: usize,
    /// The box contents, on the heap. Same wire decision as `bag_ready`
    /// above and the same one as the hearth list: `DeployRec` is what the
    /// deploy-sync packet mirrors, so a box's twelve stacks may not ride
    /// on it.
    boxes: Box<BoxStore>,
}

impl Deploys {
    pub fn new() -> Self {
        Self {
            entries: [DeployRec::default(); MAX_DEPLOYS],
            len: 0,
            bag_ready: [0; MAX_DEPLOYS],
            hearths: [HearthRec::default(); MAX_HEARTHS],
            hearth_count: 0,
            boxes: Box::new(BoxStore::new()),
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn entries(&self) -> &[DeployRec] {
        &self.entries[..self.len]
    }

    /// The live half of `bag_ready`, index-aligned to `entries()`. Read by
    /// `state_hash` (it is sim state) and by the gates that assert a bag was
    /// actually spent.
    pub fn bag_ready(&self) -> &[u64] {
        &self.bag_ready[..self.len]
    }

    pub fn hearths(&self) -> &[HearthRec] {
        &self.hearths[..self.hearth_count]
    }

    /// The live box records. Read by `state_hash` (contents are sim
    /// state) and by the gates.
    pub fn boxes(&self) -> &[BoxRec] {
        &self.boxes.entries[..self.boxes.len]
    }

    /// Resolve a packed box address to an index into `boxes()`, or `None`
    /// when no box stands there. The index is transient — swap-remove
    /// invalidates it — which is exactly why the handle on the wire is the
    /// address and never this.
    ///
    /// Handle 0 is not a box, here or anywhere: it is what every layer
    /// already says when it means *no container open* (`bridge.rs`'s
    /// "nothing is open", `core.rs`'s server-side close, `CONT_SELF`'s
    /// zeroed handle field). `box_key(0, 0, 0)` packs to 0 and would
    /// otherwise be a real address, so the two readings collide on one
    /// cell — and the client, unable to tell them apart, has to refuse a
    /// ground handle of 0 outright. The collision is closed on both sides
    /// the way `Backpacks` already closes it: **the handle is never
    /// minted** (`place_deploy` refuses a box at that address with
    /// `REFUSE_D_SPOT`, as `next_id: 1` refuses it for bags) and **the
    /// decode guards it** (here, as `index_of_id` does). Either half alone
    /// leaves one side unable to trust the other.
    pub fn box_index(&self, key: u32) -> Option<usize> {
        if key == 0 {
            return None;
        }
        self.boxes.entries[..self.boxes.len]
            .iter()
            .position(|b| box_key(b.cx, b.cz, b.level) == key)
    }

    /// One slot of box `i`. Total: an index or slot out of range reads
    /// empty, the same posture `Backpacks::slot` takes, so a stale address
    /// can never panic the sim thread.
    pub fn box_slot(&self, i: usize, s: usize) -> ItemStack {
        if i >= self.boxes.len || s >= BOX_SLOTS {
            return ItemStack::default();
        }
        self.boxes.entries[i].items[s]
    }

    /// Write one slot of box `i`. Total in the same way: out of range
    /// writes nothing.
    pub fn set_box_slot(&mut self, i: usize, s: usize, stack: ItemStack) {
        if i >= self.boxes.len || s >= BOX_SLOTS {
            return;
        }
        self.boxes.entries[i].items[s] = stack;
    }

    /// Planar distance from a box to a player, against the same
    /// `BUILD_REACH_M` a door uses — and planar for the same reason: a
    /// door on the storey above is reachable today too, and inventing a
    /// vertical rule for boxes alone would make two verbs disagree about
    /// what "in reach" means. Level is part of the *address*, not of the
    /// reach test.
    pub fn box_in_reach(&self, i: usize, p: &Player) -> bool {
        if i >= self.boxes.len {
            return false;
        }
        let b = self.boxes.entries[i];
        let (bx, bz) = cell_center(b.cx, b.cz);
        let (px, pz) = player_xz(p);
        let (dx, dz) = (bx - px, bz - pz);
        dx * dx + dz * dz <= crate::build::BUILD_REACH_M * crate::build::BUILD_REACH_M
    }

    /// How many spills this tick's removals left. Read once per tick by
    /// `World::step`, which owns the bag store and the clock.
    pub fn box_spill_len(&self) -> usize {
        self.boxes.spill_len
    }

    /// One parked spill. Read by value — the caller stands a bag up from
    /// it and never holds a reference into the store.
    pub fn box_spill_at(&self, i: usize) -> BoxRec {
        if i >= self.boxes.spill_len {
            return BoxRec::default();
        }
        self.boxes.spill[i]
    }

    /// Empty the spill buffer, after `World::step` has stood every parked
    /// record up. Separate from the reads so a spill is drained exactly
    /// once and never half-drained by an early return.
    pub fn clear_box_spill(&mut self) {
        self.boxes.spill_len = 0;
    }

    pub fn find(&self, cx: u16, cz: u16, level: u8, loc: u8) -> Option<&DeployRec> {
        self.entries[..self.len]
            .iter()
            .find(|d| d.cx == cx && d.cz == cz && d.level == level && d.loc == loc)
    }

    fn insert(&mut self, rec: DeployRec) -> bool {
        if self.len == MAX_DEPLOYS {
            return false;
        }
        self.entries[self.len] = rec;
        // A bag is born ready: place one and the next death is answered.
        self.bag_ready[self.len] = 0;
        self.len += 1;
        true
    }

    /// Swap-remove entry `i`; drops the hearth record too when the entry
    /// was a hearth. The caller announces the removal event.
    fn remove_at(&mut self, i: usize, dc: &DeployContent) {
        let rec = self.entries[i];
        self.len -= 1;
        self.entries[i] = self.entries[self.len];
        // Both halves move together or the cooldown array stops describing
        // the record it is indexed against.
        self.bag_ready[i] = self.bag_ready[self.len];
        if dc.defs[rec.row as usize].arch == ARCH_HEARTH {
            if let Some(h) = self.hearths[..self.hearth_count]
                .iter()
                .position(|h| h.cx == rec.cx && h.cz == rec.cz && h.level == rec.level)
            {
                self.hearth_count -= 1;
                self.hearths[h] = self.hearths[self.hearth_count];
            }
        }
        if dc.defs[rec.row as usize].arch == ARCH_BOX {
            if let Some(b) = self.box_index(box_key(rec.cx, rec.cz, rec.level)) {
                let bx = self.boxes.entries[b];
                self.boxes.len -= 1;
                let last = self.boxes.len;
                self.boxes.entries[b] = self.boxes.entries[last];
                self.boxes.entries[last] = BoxRec::default();
                // What was inside outlives the box. Parked rather than
                // dropped where it stands, because this path is reached
                // from decay and from a raid and holds neither the bag
                // store nor the clock; `World::step` drains it this same
                // tick. An empty box parks nothing — `stand_up` would
                // refuse it anyway, and a full buffer is then never spent
                // on boxes that had nothing in them.
                if !bx.is_empty() && self.boxes.spill_len < MAX_BOX_SPILL_PER_TICK {
                    self.boxes.spill[self.boxes.spill_len] = bx;
                    self.boxes.spill_len += 1;
                }
            }
        }
    }

    /// First hearth (list order) whose radius covers the planar point.
    /// `require_stock` skips empty hearths (decay coverage); the claim
    /// checks pass false (an empty foreign hearth still claims).
    fn covering_hearth(&self, x: f32, z: f32, require_stock: bool) -> Option<usize> {
        let r2 = HEARTH_RADIUS_M * HEARTH_RADIUS_M;
        self.hearths[..self.hearth_count].iter().position(|h| {
            if require_stock && h.stock.iter().all(|&s| s == 0) {
                return false;
            }
            let (hx, hz) = cell_center(h.cx, h.cz);
            let (dx, dz) = (hx - x, hz - z);
            dx * dx + dz * dz <= r2
        })
    }

    /// Whether a *foreign* hearth claims the planar point — the privilege
    /// wall piece and deploy placement both check.
    pub fn foreign_claim(&self, x: f32, z: f32, placer: u32) -> bool {
        let r2 = HEARTH_RADIUS_M * HEARTH_RADIUS_M;
        self.hearths[..self.hearth_count].iter().any(|h| {
            if h.owner == placer {
                return false;
            }
            let (hx, hz) = cell_center(h.cx, h.cz);
            let (dx, dz) = (hx - x, hz - z);
            dx * dx + dz * dz <= r2
        })
    }

    /// Whether any placed deployable of `arch` sits within `radius_m`
    /// (planar) of the point — the craft station check.
    pub fn arch_near(&self, dc: &DeployContent, arch: u8, x: f32, z: f32, radius_m: f32) -> bool {
        let r2 = radius_m * radius_m;
        self.entries[..self.len].iter().any(|d| {
            if dc.defs[d.row as usize].arch != arch {
                return false;
            }
            let (ax, az) = cell_center(d.cx, d.cz);
            let (dx, dz) = (ax - x, az - z);
            dx * dx + dz * dz <= r2
        })
    }

    fn own_bag_count(&self, dc: &DeployContent, owner: u32) -> usize {
        self.entries[..self.len]
            .iter()
            .filter(|d| d.owner == owner && dc.defs[d.row as usize].arch == ARCH_BAG)
            .count()
    }

    /// The bag a dying player wakes on, and the cooldown that spends it.
    ///
    /// Scans `owner`'s own ready bags and returns the planar cell center of
    /// the one **nearest the point given** — the body's position as it
    /// fell, so a death defending a compound wakes on the bag inside that
    /// compound rather than on one across the island. Ties go to the
    /// earlier entry, which is the store's insertion order and therefore
    /// the same on every replay of the same log.
    ///
    /// The chosen bag is *spent*: its `bag_ready` entry moves to
    /// `tick + BAG_COOLDOWN_TICKS`, and until then every scan skips it. So
    /// a player killed twice inside the cooldown wakes on a second bag if
    /// they placed one and back on the spawn ring if they did not — which
    /// is the whole reason the cooldown exists, and the reason `BAG_CAP`
    /// bounds how many answers a defender can stack.
    ///
    /// `None` means exactly one thing: no bag of this owner's is ready.
    /// The caller falls back to the ring on it.
    ///
    /// Bounded and allocation-free — one pass of the store, squared planar
    /// distance (monotone in distance, so no `sqrt` and no trig), and no
    /// iteration order that is not the store's own (wall 1).
    pub fn claim_bag(
        &mut self,
        dc: &DeployContent,
        owner: u32,
        x: f32,
        z: f32,
        tick: u64,
    ) -> Option<(f32, f32)> {
        let mut best: Option<(usize, f32)> = None;
        for (i, d) in self.entries[..self.len].iter().enumerate() {
            if d.owner != owner
                || dc.defs[d.row as usize].arch != ARCH_BAG
                || self.bag_ready[i] > tick
            {
                continue;
            }
            let (bx, bz) = cell_center(d.cx, d.cz);
            let (dx, dz) = (bx - x, bz - z);
            let d2 = dx * dx + dz * dz;
            if best.is_none_or(|(_, b)| d2 < b) {
                best = Some((i, d2));
            }
        }
        let (i, _) = best?;
        self.bag_ready[i] = tick.saturating_add(BAG_COOLDOWN_TICKS);
        Some(cell_center(self.entries[i].cx, self.entries[i].cz))
    }
}

impl Default for Deploys {
    fn default() -> Self {
        Self::new()
    }
}

/// Where a broken box's contents fall: the centre of its own cell, on its
/// own storey. The height is the collider's floor formula (`collide.rs`:
/// terrain under the cell plus `level * LEVEL_H_M`), so the bag lands on
/// the floor the box was standing on rather than inside it.
pub fn box_drop_pos(seed: u64, cx: u16, cz: u16, level: u8) -> (f32, f32, f32) {
    let (x, z) = cell_center(cx, cz);
    (x, terrain::height(seed, x, z) + level as f32 * LEVEL_H_M, z)
}

fn cell_center(cx: u16, cz: u16) -> (f32, f32) {
    (
        cx as f32 * crate::build::BUILD_CELL_M + crate::build::BUILD_CELL_M * 0.5,
        cz as f32 * crate::build::BUILD_CELL_M + crate::build::BUILD_CELL_M * 0.5,
    )
}

fn player_xz(p: &Player) -> (f32, f32) {
    (
        p.body.qx as f32 * crate::movement::POS_XZ_Q,
        p.body.qz as f32 * crate::movement::POS_XZ_Q,
    )
}

/// Whether `loc` is the kind of slot the placement class occupies.
fn loc_fits_placement(placement: u8, loc: u8) -> bool {
    match placement {
        PLACE_DOORWAY => loc == LOC_EDGE_W || loc == LOC_EDGE_N,
        PLACE_GROUND | PLACE_FOUNDATION | PLACE_ANY => loc == LOC_PLANE,
        _ => false,
    }
}

/// Ground-class terrain rule: same buildable shape as a foundation
/// (build.rs consts), and the cell body must be piece-free.
fn ground_ok(seed: u64, pieces: &Pieces, cx: u16, cz: u16) -> bool {
    if pieces.find(cx, cz, 0, LOC_PLANE).is_some() {
        return false;
    }
    let (x, z) = cell_center(cx, cz);
    terrain::height(seed, x, z) >= crate::build::FOUNDATION_MIN_H_M
        && terrain::slope(seed, x, z) < crate::build::FOUNDATION_MAX_SLOPE
}

/// Apply one deploy-place request (`Command::PlaceDeploy`). Refusals are
/// events, not errors. The deployable's own item is the cost, consumed
/// whole; EV_DEPLOY_PLACED announces the record for the wire.
#[allow(clippy::too_many_arguments)]
pub fn place_deploy(
    seed: u64,
    dc: &DeployContent,
    bc: &BuildContent,
    pieces: &mut Pieces,
    deploys: &mut Deploys,
    p: &mut Player,
    tick: u64,
    row: u16,
    cx: u16,
    cz: u16,
    level: u8,
    loc: u8,
    events: &mut EventQueue,
) {
    if row >= dc.def_count {
        events.push(EV_DEPLOY_REFUSED, p.id, REFUSE_D_KIND, 0);
        return;
    }
    let def = &dc.defs[row as usize];
    if def.hp == 0 {
        events.push(EV_DEPLOY_REFUSED, p.id, REFUSE_D_KIND, 0);
        return;
    }
    if (cx as usize) >= MAX_BUILD_COORD
        || (cz as usize) >= MAX_BUILD_COORD
        || (level as usize) >= MAX_BUILD_LEVELS
        || !loc_fits_placement(def.placement, loc)
        || deploys.find(cx, cz, level, loc).is_some()
        // The one address a box may not have: `box_key(0, 0, 0)` is 0, and
        // 0 is the reserved "no container" handle (`box_index`). Refusing
        // it here is the minting half of that pair — a box placed at this
        // one cell could be opened by nobody, and a spot that refuses is
        // strictly better than an item store that swallows. It costs one
        // build cell at the world's origin corner and nothing else; every
        // other level of that cell, and every other cell, is unaffected.
        || (def.arch == ARCH_BOX && box_key(cx, cz, level) == 0)
    {
        events.push(EV_DEPLOY_REFUSED, p.id, REFUSE_D_SPOT, 0);
        return;
    }
    let (ax, az) = cell_center(cx, cz);
    let (px, pz) = player_xz(p);
    let (dx, dz) = (ax - px, az - pz);
    if dx * dx + dz * dz > crate::build::BUILD_REACH_M * crate::build::BUILD_REACH_M {
        events.push(EV_DEPLOY_REFUSED, p.id, REFUSE_D_REACH, 0);
        return;
    }
    if deploys.foreign_claim(ax, az, p.id) {
        events.push(EV_DEPLOY_REFUSED, p.id, REFUSE_D_CLAIM, 0);
        return;
    }
    let supported = match def.placement {
        PLACE_GROUND => level == 0 && ground_ok(seed, pieces, cx, cz),
        PLACE_FOUNDATION => pieces.find(cx, cz, level, LOC_PLANE).is_some(),
        PLACE_ANY => {
            pieces.find(cx, cz, level, LOC_PLANE).is_some()
                || (level == 0 && ground_ok(seed, pieces, cx, cz))
        }
        PLACE_DOORWAY => pieces
            .find(cx, cz, level, loc)
            .is_some_and(|r| bc.pieces[r.row as usize].shape == SHAPE_DOORWAY),
        _ => false,
    };
    if !supported {
        let reason = if def.placement == PLACE_GROUND && level == 0 {
            REFUSE_D_TERRAIN
        } else {
            REFUSE_D_SUPPORT
        };
        events.push(EV_DEPLOY_REFUSED, p.id, reason, 0);
        return;
    }
    if def.arch == ARCH_HEARTH {
        // No hearth inside any hearth's radius (own included), and the
        // dense hearth list is a hard cap.
        if deploys.covering_hearth(ax, az, false).is_some() {
            events.push(EV_DEPLOY_REFUSED, p.id, REFUSE_D_OVERLAP, 0);
            return;
        }
        if deploys.hearth_count == MAX_HEARTHS {
            events.push(EV_DEPLOY_REFUSED, p.id, REFUSE_D_FULL, 0);
            return;
        }
    }
    if def.arch == ARCH_BAG && deploys.own_bag_count(dc, p.id) >= BAG_CAP {
        events.push(EV_DEPLOY_REFUSED, p.id, REFUSE_D_BAG_CAP, 0);
        return;
    }
    // Checked before the insert for the same reason the hearth cap is: the
    // record below cannot be written into a full store, so the append that
    // follows the insert needs no second guard.
    if def.arch == ARCH_BOX && deploys.boxes.len == MAX_BOXES {
        events.push(EV_DEPLOY_REFUSED, p.id, REFUSE_D_FULL, 0);
        return;
    }
    if inv_count(&p.inv, def.item) < 1 {
        events.push(EV_DEPLOY_REFUSED, p.id, REFUSE_D_COST, 0);
        return;
    }
    let rec = DeployRec {
        cx,
        cz,
        level,
        loc,
        row: row as u8,
        owner: p.id,
        hp: def.hp,
        uh: (tick / UPKEEP_PERIOD_TICKS) as u16,
        open: false,
        // A door is born locked to the hand that placed it (lock v0): the
        // base is the point of the base. Everything else ignores the bit.
        locked: def.arch == ARCH_DOOR,
    };
    if !deploys.insert(rec) {
        events.push(EV_DEPLOY_REFUSED, p.id, REFUSE_D_FULL, 0);
        return;
    }
    if def.arch == ARCH_DOOR {
        // Doors place closed and seal their doorway (door v0).
        pieces.set_door(cx, cz, level, loc, true);
    }
    if def.arch == ARCH_HEARTH {
        deploys.hearths[deploys.hearth_count] = HearthRec {
            cx,
            cz,
            level,
            owner: p.id,
            stock: [0; HEARTH_STOCK_ROWS],
        };
        deploys.hearth_count += 1;
    }
    if def.arch == ARCH_BOX {
        let n = deploys.boxes.len;
        deploys.boxes.entries[n] = BoxRec {
            cx,
            cz,
            level,
            owner: p.id,
            items: [ItemStack::default(); BOX_SLOTS],
        };
        deploys.boxes.len += 1;
    }
    inv_take(&mut p.inv, def.item, 1);
    events.push(
        EV_DEPLOY_PLACED,
        crate::gather::cell_key(cx, cz),
        ((level as u32) << 16) | ((loc as u32) << 8) | row as u32,
        p.id,
    );
}

/// Apply one feed request (`Command::Feed`): move up to `FEED_CHUNK`
/// units per upkeep material from the feeder into the hearth at the
/// address, capped at `STOCK_MAX` per row. Any hearth in reach accepts
/// (a gift is not a grief); EV_STOCK acks the feeder with the address —
/// the wire reads the new stock from the world when it encodes.
pub fn feed(
    dc: &DeployContent,
    deploys: &mut Deploys,
    p: &mut Player,
    cx: u16,
    cz: u16,
    level: u8,
    events: &mut EventQueue,
) {
    let (ax, az) = cell_center(cx, cz);
    let (px, pz) = player_xz(p);
    let (dx, dz) = (ax - px, az - pz);
    let in_reach = dx * dx + dz * dz <= crate::build::BUILD_REACH_M * crate::build::BUILD_REACH_M;
    let Some(h) = deploys.hearths[..deploys.hearth_count]
        .iter()
        .position(|h| h.cx == cx && h.cz == cz && h.level == level)
    else {
        events.push(EV_DEPLOY_REFUSED, p.id, REFUSE_D_HEARTH, 0);
        return;
    };
    if !in_reach {
        events.push(EV_DEPLOY_REFUSED, p.id, REFUSE_D_REACH, 0);
        return;
    }
    for m in 0..dc.mat_count as usize {
        let item = dc.mats[m];
        let room = STOCK_MAX.saturating_sub(deploys.hearths[h].stock[m]);
        let want = FEED_CHUNK.min(room).min(inv_count(&p.inv, item));
        if want > 0 {
            let took = inv_take(&mut p.inv, item, want);
            deploys.hearths[h].stock[m] += took;
        }
    }
    events.push(
        EV_STOCK,
        p.id,
        crate::gather::cell_key(cx, cz),
        level as u32,
    );
}

/// The door at the address, if one is there and the player stands within
/// build reach of its cell center. Both refusals are pushed here, so the
/// use and lock paths bounce identically for the same reason.
#[allow(clippy::too_many_arguments)]
fn door_in_reach(
    dc: &DeployContent,
    deploys: &Deploys,
    p: &Player,
    cx: u16,
    cz: u16,
    level: u8,
    loc: u8,
    events: &mut EventQueue,
) -> Option<usize> {
    let hit = deploys.entries[..deploys.len]
        .iter()
        .position(|d| d.cx == cx && d.cz == cz && d.level == level && d.loc == loc)
        .filter(|&i| dc.defs[deploys.entries[i].row as usize].arch == ARCH_DOOR);
    let Some(i) = hit else {
        events.push(EV_DEPLOY_REFUSED, p.id, REFUSE_D_DOOR, 0);
        return None;
    };
    let (ax, az) = cell_center(cx, cz);
    let (px, pz) = player_xz(p);
    let (dx, dz) = (ax - px, az - pz);
    if dx * dx + dz * dz > crate::build::BUILD_REACH_M * crate::build::BUILD_REACH_M {
        events.push(EV_DEPLOY_REFUSED, p.id, REFUSE_D_REACH, 0);
        return None;
    }
    Some(i)
}

/// EV_DOOR: the door's whole state after a change, absolute (world.rs
/// documents the packing). One announcement serves the toggle and the
/// lock, so a client never holds half a door.
fn announce_door(deploys: &Deploys, i: usize, by: u32, events: &mut EventQueue) {
    let d = &deploys.entries[i];
    events.push(
        EV_DOOR,
        crate::gather::cell_key(d.cx, d.cz),
        ((d.level as u32) << 16) | ((d.loc as u32) << 8) | ((d.locked as u32) << 1) | d.open as u32,
        by,
    );
}

/// Apply one use request (`Command::Use`): toggle the door at the
/// address. Any player within build reach may toggle an **unlocked**
/// door; a locked one answers only to its owner (lock v0) and bounces
/// every other hand with `REFUSE_D_OWNER`. The shut bit in the collision
/// index flips in the same call, so the tick that toggles is the tick
/// that blocks (or opens); EV_DOOR announces the new state for the wire.
#[allow(clippy::too_many_arguments)]
pub fn use_door(
    dc: &DeployContent,
    pieces: &mut Pieces,
    deploys: &mut Deploys,
    p: &mut Player,
    cx: u16,
    cz: u16,
    level: u8,
    loc: u8,
    events: &mut EventQueue,
) {
    let Some(i) = door_in_reach(dc, deploys, p, cx, cz, level, loc, events) else {
        return;
    };
    if deploys.entries[i].locked && deploys.entries[i].owner != p.id {
        events.push(EV_DEPLOY_REFUSED, p.id, REFUSE_D_OWNER, 0);
        return;
    }
    let open = !deploys.entries[i].open;
    deploys.entries[i].open = open;
    pieces.set_door(cx, cz, level, loc, !open);
    announce_door(deploys, i, p.id, events);
}

/// Apply one lock request (`Command::Lock`): set the door's lock bit to
/// `locked`. Owner-only, and absolute rather than a toggle — two presses
/// racing would otherwise fight over the state (the same reason the use
/// action carries no state). The leaf never moves: locking an open door
/// leaves it open, and only its owner can then shut it.
#[allow(clippy::too_many_arguments)]
pub fn set_lock(
    dc: &DeployContent,
    deploys: &mut Deploys,
    p: &Player,
    cx: u16,
    cz: u16,
    level: u8,
    loc: u8,
    locked: bool,
    events: &mut EventQueue,
) {
    let Some(i) = door_in_reach(dc, deploys, p, cx, cz, level, loc, events) else {
        return;
    };
    if deploys.entries[i].owner != p.id {
        events.push(EV_DEPLOY_REFUSED, p.id, REFUSE_D_OWNER, 0);
        return;
    }
    deploys.entries[i].locked = locked;
    announce_door(deploys, i, p.id, events);
}

/// Per-period charge for one cost row: `ceil(cost × pct / 100 / 24)`.
fn charge_of(cost: u16, pct: u16) -> u32 {
    let num = cost as u32 * pct as u32;
    num.div_ceil(100 * PERIODS_PER_DAY)
}

/// Per-period decay for a def's max hp: `max(1, maxhp × pct / 100)`.
fn decay_of(max_hp: u16) -> u16 {
    ((max_hp as u32 * DECAY_PCT_PER_PERIOD) / 100).max(1) as u16
}

/// Remove the piece at store index `i` and the deployable standing at the
/// exact same address (the door in the doorway, the box on the floor),
/// broadcasting both removals. **The one removal path**: decay reaches it
/// through the sweep, a raid through `damage_piece`, and neither can grow
/// a cascade the other lacks.
///
/// This is the single removal, not the structural one: a caller that took
/// the piece out of the world follows it with `build::collapse_from` so
/// what rested on it comes down too. `collapse_from` itself calls this and
/// must not, which is why the two are separate functions.
pub(crate) fn drop_piece(
    dc: &DeployContent,
    pieces: &mut Pieces,
    deploys: &mut Deploys,
    i: usize,
    shape: u8,
    events: &mut EventQueue,
) {
    let rec = pieces.entries()[i];
    pieces.remove_at(i, shape);
    events.push(
        EV_PIECE_REMOVED,
        crate::gather::cell_key(rec.cx, rec.cz),
        ((rec.level as u32) << 16) | ((rec.loc as u32) << 8) | rec.row as u32,
        0,
    );
    if let Some(di) = deploys.entries[..deploys.len]
        .iter()
        .position(|d| d.cx == rec.cx && d.cz == rec.cz && d.level == rec.level && d.loc == rec.loc)
    {
        drop_deploy(dc, pieces, deploys, di, events);
    }
}

/// Remove the deployable at store index `di`, unsealing its doorway if it
/// was a door, and broadcast the removal. The other half of the one
/// removal path.
fn drop_deploy(
    dc: &DeployContent,
    pieces: &mut Pieces,
    deploys: &mut Deploys,
    di: usize,
    events: &mut EventQueue,
) {
    let rec = deploys.entries[di];
    deploys.remove_at(di, dc);
    if dc.defs[rec.row as usize].arch == ARCH_DOOR {
        pieces.set_door(rec.cx, rec.cz, rec.level, rec.loc, false);
    }
    events.push(
        EV_DEPLOY_REMOVED,
        crate::gather::cell_key(rec.cx, rec.cz),
        ((rec.level as u32) << 16) | ((rec.loc as u32) << 8) | rec.row as u32,
        0,
    );
}

/// Take `amount` hp off the piece at store index `i` — the raid verb's
/// write (combat.rs picks the target, this owns the store). Broadcasts
/// `EV_STRUCT_HIT` with the hp left, then removes through `drop_piece` at
/// zero. Returns true when the piece fell.
///
/// hp is otherwise sim-only (build.rs) — a raid is the one case where the
/// number has to cross, because a wall that shows no progress under thirty
/// swings reads as an invulnerable wall.
#[allow(clippy::too_many_arguments)]
pub fn damage_piece(
    dc: &DeployContent,
    bc: &BuildContent,
    pieces: &mut Pieces,
    deploys: &mut Deploys,
    i: usize,
    amount: u16,
    budget: &mut usize,
    events: &mut EventQueue,
) -> bool {
    let rec = pieces.entries()[i];
    let mut left = rec.hp.saturating_sub(amount);
    if left == 0 && *budget == 0 {
        // The tick's removal budget is spent (limits.rs
        // `MAX_REMOVALS_PER_TICK`), so the wall stops at its last hp and
        // the next swing takes it. Deferring here rather than after the
        // subtraction is what keeps a standing piece's hp at 1 or more:
        // parking a 0-hp piece in the store would leave a wall that no
        // sweep removes and every swing re-reports as already broken.
        // The raider is told the truth — this hit reads as 0 dealt.
        left = 1;
    }
    let dealt = rec.hp - left;
    events.push(
        EV_STRUCT_HIT,
        crate::gather::cell_key(rec.cx, rec.cz),
        ((rec.level as u32) << 16) | ((rec.loc as u32) << 8) | rec.row as u32,
        ((dealt as u32) << 16) | left as u32,
    );
    if left == 0 {
        drop_piece(
            dc,
            pieces,
            deploys,
            i,
            bc.pieces[rec.row as usize].shape,
            events,
        );
        *budget -= 1;
        // Take the legs out and the base comes down: everything that
        // rested on this address is re-checked against the same support
        // rule that let it be placed (build.rs).
        crate::build::collapse_from(
            dc,
            bc,
            pieces,
            deploys,
            (rec.cx, rec.cz, rec.level, rec.loc),
            budget,
            events,
        );
        return true;
    }
    pieces.set_hp(i, left);
    false
}

/// Take `amount` hp off the deployable at store index `di`. The deployable
/// half of `damage_piece`, `STRUCT_DEPLOY_BIT` set on the wire so a client
/// knows which store the address names.
pub fn damage_deploy(
    dc: &DeployContent,
    pieces: &mut Pieces,
    deploys: &mut Deploys,
    di: usize,
    amount: u16,
    events: &mut EventQueue,
) -> bool {
    let rec = deploys.entries[di];
    let left = rec.hp.saturating_sub(amount);
    let dealt = rec.hp - left;
    events.push(
        EV_STRUCT_HIT,
        crate::gather::cell_key(rec.cx, rec.cz),
        STRUCT_DEPLOY_BIT | ((rec.level as u32) << 16) | ((rec.loc as u32) << 8) | rec.row as u32,
        ((dealt as u32) << 16) | left as u32,
    );
    if left == 0 {
        drop_deploy(dc, pieces, deploys, di, events);
        return true;
    }
    deploys.entries[di].hp = left;
    false
}

/// One tick of the upkeep/decay sweep: advance each store's cursor by
/// `UPKEEP_SWEEP_PER_TICK` entries, processing due periods per entry
/// (charge from a covering hearth or decay; removal at 0 hp). Bounded
/// work, deterministic order (cursor + list order). Piece removal also
/// removes the deployable at the exact same address (the door in the
/// decayed doorway, the box on the decayed floor).
#[allow(clippy::too_many_arguments)]
pub fn upkeep_sweep(
    dc: &DeployContent,
    bc: &BuildContent,
    pieces: &mut Pieces,
    deploys: &mut Deploys,
    tick: u64,
    piece_cursor: &mut u32,
    deploy_cursor: &mut u32,
    budget: &mut usize,
    events: &mut EventQueue,
) {
    if dc.mat_count == 0 {
        return; // no upkeep materials priced: decay is off (inert table)
    }
    let h_now = (tick / UPKEEP_PERIOD_TICKS) as u16;

    // --- pieces ---------------------------------------------------------
    let mut visits = 0usize;
    while visits < UPKEEP_SWEEP_PER_TICK && !pieces.is_empty() {
        visits += 1;
        let i = (*piece_cursor as usize) % pieces.len();
        *piece_cursor = ((i + 1) % pieces.len()) as u32;
        let rec = pieces.entries()[i];
        if rec.uh >= h_now {
            continue;
        }
        let def = bc.pieces[rec.row as usize];
        let (x, z) = cell_center(rec.cx, rec.cz);
        let mut hp = rec.hp;
        let mut uh = rec.uh;
        let mut removed = false;
        let mut steps = 0u32;
        while uh < h_now && steps < SWEEP_CATCHUP_MAX {
            steps += 1;
            uh += 1;
            // First hearth in list order that can pay the whole charge.
            let payer = deploys.hearths[..deploys.hearth_count]
                .iter()
                .position(|hr| {
                    let (hx, hz) = cell_center(hr.cx, hr.cz);
                    let (dx, dz) = (hx - x, hz - z);
                    if dx * dx + dz * dz > HEARTH_RADIUS_M * HEARTH_RADIUS_M {
                        return false;
                    }
                    (0..dc.mat_count as usize).all(|m| {
                        let due: u32 = def
                            .costs
                            .iter()
                            .take(def.n_costs as usize)
                            .filter(|&&(item, _)| item == dc.mats[m])
                            .map(|&(_, cost)| charge_of(cost, dc.upkeep_pct_per_day))
                            .sum();
                        hr.stock[m] >= due
                    })
                });
            if let Some(hi) = payer {
                for m in 0..dc.mat_count as usize {
                    let due: u32 = def
                        .costs
                        .iter()
                        .take(def.n_costs as usize)
                        .filter(|&&(item, _)| item == dc.mats[m])
                        .map(|&(_, cost)| charge_of(cost, dc.upkeep_pct_per_day))
                        .sum();
                    deploys.hearths[hi].stock[m] -= due;
                }
            } else {
                let d = decay_of(def.hp);
                if hp <= d {
                    hp = 0;
                    removed = true;
                    break;
                }
                hp -= d;
            }
        }
        if steps >= SWEEP_CATCHUP_MAX {
            uh = h_now; // bounded catch-up: the missed hours are forgiven
        }
        if removed {
            if *budget == 0 {
                // The tick's removal budget is spent (limits.rs
                // `MAX_REMOVALS_PER_TICK`). Defer *before* the removal and
                // before `set_upkeep`, so this entry keeps the hp and the
                // upkeep hour it came in with and the whole computation
                // above is simply redone next tick. Rewind the cursor onto
                // this entry rather than past it: the sweep advances by
                // visiting, and a deferred entry has not been served.
                //
                // `break`, not `return`: only the piece half spends this
                // budget. Deployable decay below removes at most one
                // record per visit with no cascade under it, so it is
                // already bounded by `UPKEEP_SWEEP_PER_TICK` and stopping
                // it here would be a second, unstated cap.
                *piece_cursor = i as u32;
                break;
            }
            // Same removal path a raid takes — cascade included, and the
            // structural collapse with it: a floor whose wall decayed out
            // from under it falls exactly as one a raider broke would.
            drop_piece(dc, pieces, deploys, i, def.shape, events);
            *budget -= 1;
            crate::build::collapse_from(
                dc,
                bc,
                pieces,
                deploys,
                (rec.cx, rec.cz, rec.level, rec.loc),
                budget,
                events,
            );
            *piece_cursor = (i as u32).min(pieces.len().saturating_sub(1) as u32);
        } else {
            pieces.set_upkeep(i, hp, uh);
        }
    }

    // --- deployables ----------------------------------------------------
    let mut visits = 0usize;
    while visits < UPKEEP_SWEEP_PER_TICK && deploys.len > 0 {
        visits += 1;
        let i = (*deploy_cursor as usize) % deploys.len;
        *deploy_cursor = ((i + 1) % deploys.len) as u32;
        let rec = deploys.entries[i];
        if rec.uh >= h_now {
            continue;
        }
        let def = dc.defs[rec.row as usize];
        let (x, z) = cell_center(rec.cx, rec.cz);
        let mut hp = rec.hp;
        let mut uh = rec.uh;
        let mut removed = false;
        let mut steps = 0u32;
        while uh < h_now && steps < SWEEP_CATCHUP_MAX {
            steps += 1;
            uh += 1;
            // Covered by any stocked hearth ⇒ free; uncovered ⇒ decay.
            // (An empty hearth covers nothing, itself included.)
            if deploys.covering_hearth(x, z, true).is_none() {
                let d = decay_of(def.hp);
                if hp <= d {
                    hp = 0;
                    removed = true;
                    break;
                }
                hp -= d;
            }
        }
        if steps >= SWEEP_CATCHUP_MAX {
            uh = h_now;
        }
        if removed {
            drop_deploy(dc, pieces, deploys, i, events);
            *deploy_cursor = (i as u32).min(deploys.len.saturating_sub(1) as u32);
        } else {
            deploys.entries[i].hp = hp;
            deploys.entries[i].uh = uh;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build::{BuildContent, Pieces, LOC_PLANE};
    use crate::gather::ItemStack;
    use crate::limits::MAX_REMOVALS_PER_TICK;

    /// One tick's structural removal budget, as `World::tick` hands it
    /// out — these fixtures never approach it (build.rs owns the tests
    /// that do).
    fn tick_budget() -> usize {
        MAX_REMOVALS_PER_TICK
    }

    use crate::movement::Body;
    use crate::world::{EventQueue, Player};

    const SEED: u64 = 20260731;
    const CX: u16 = 341;
    const CZ: u16 = 341;

    fn player_at_cell(cx: u16, cz: u16, items: &[(u16, u16)]) -> Player {
        let mut p = Player {
            id: 7,
            active: true,
            body: Body::at(
                SEED,
                (cx as f32 + 0.5) * crate::build::BUILD_CELL_M,
                (cz as f32 + 0.5) * crate::build::BUILD_CELL_M,
            ),
            ..Player::default()
        };
        for (i, &(item, count)) in items.iter().enumerate() {
            p.inv[i] = ItemStack { item, count };
        }
        p
    }

    fn last(ev: &EventQueue) -> (u8, u32, u32, u32) {
        let e = ev.entries()[ev.len() - 1];
        (e.code, e.a, e.b, e.c)
    }

    /// A foundation at (cx, cz) so foundation-class placement has support.
    fn founded(bc: &BuildContent, pieces: &mut Pieces, p: &mut Player, cx: u16, cz: u16) {
        let mut ev = EventQueue::default();
        crate::build::place(
            SEED,
            bc,
            &Deploys::new(),
            pieces,
            p,
            0,
            0,
            cx,
            cz,
            0,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(last(&ev).0, crate::world::EV_PIECE_PLACED);
    }

    #[test]
    fn placement_classes_enforce_their_support() {
        let dc = DeployContent::probe_fixture();
        let bc = BuildContent::probe_fixture();
        let mut pieces = Pieces::new();
        let mut deploys = Deploys::new();
        let mut ev = EventQueue::default();
        let mut p = player_at_cell(CX, CZ, &[(0, 99), (1, 99), (2, 5), (3, 5), (4, 5), (5, 5)]);

        // Hearth (foundation class) on bare terrain refuses.
        place_deploy(
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
        assert_eq!(last(&ev).2, REFUSE_D_SUPPORT);

        // Bag (ground class) on bare terrain places and pays its item.
        place_deploy(
            SEED,
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut p,
            0,
            3,
            CX,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(last(&ev).0, crate::world::EV_DEPLOY_PLACED);
        assert_eq!(inv_count(&p.inv, 5), 4, "bag item consumed");

        // Occupied address refuses.
        place_deploy(
            SEED,
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut p,
            0,
            3,
            CX,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(last(&ev).2, REFUSE_D_SPOT);

        // Foundation next door, hearth on it places; ground bag on the
        // foundation cell refuses (ground means terrain).
        founded(&bc, &mut pieces, &mut p, CX + 1, CZ);
        place_deploy(
            SEED,
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut p,
            0,
            0,
            CX + 1,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(last(&ev).0, crate::world::EV_DEPLOY_PLACED);
        assert_eq!(deploys.hearths().len(), 1);
        place_deploy(
            SEED,
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut p,
            0,
            3,
            CX + 1,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(last(&ev).2, REFUSE_D_SPOT, "hearth holds the address");

        // Door needs a doorway edge piece: none ⇒ support refusal; a wall
        // there is still not a doorway.
        place_deploy(
            SEED,
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut p,
            0,
            2,
            CX + 1,
            CZ,
            0,
            LOC_EDGE_W,
            &mut ev,
        );
        assert_eq!(last(&ev).2, REFUSE_D_SUPPORT);
        crate::build::place(
            SEED,
            &bc,
            &deploys,
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
        place_deploy(
            SEED,
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut p,
            0,
            2,
            CX + 1,
            CZ,
            0,
            LOC_EDGE_W,
            &mut ev,
        );
        assert_eq!(last(&ev).2, REFUSE_D_SUPPORT, "a wall is not a doorway");

        // Bad row, wrong loc, empty pocket.
        place_deploy(
            SEED,
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut p,
            0,
            9,
            CX,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(last(&ev).2, REFUSE_D_KIND);
        place_deploy(
            SEED,
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut p,
            0,
            3,
            CX,
            CZ,
            0,
            LOC_EDGE_W,
            &mut ev,
        );
        assert_eq!(last(&ev).2, REFUSE_D_SPOT);
        let mut poor = player_at_cell(CX + 2, CZ, &[]);
        place_deploy(
            SEED,
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut poor,
            0,
            3,
            CX + 2,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(last(&ev).2, REFUSE_D_COST);
    }

    #[test]
    fn hearth_claims_and_overlap_refuse() {
        let dc = DeployContent::probe_fixture();
        let bc = BuildContent::probe_fixture();
        let mut pieces = Pieces::new();
        let mut deploys = Deploys::new();
        let mut ev = EventQueue::default();
        let mut own = player_at_cell(CX, CZ, &[(0, 99), (1, 99), (2, 9)]);
        founded(&bc, &mut pieces, &mut own, CX, CZ);
        place_deploy(
            SEED,
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut own,
            0,
            0,
            CX,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(last(&ev).0, crate::world::EV_DEPLOY_PLACED);

        // A stranger inside the radius: pieces refuse with the claim
        // reason, deploys too; outside the radius both work.
        let mut foe = player_at_cell(CX + 2, CZ, &[(0, 99), (1, 99), (5, 5)]);
        foe.id = 8;
        crate::build::place(
            SEED,
            &bc,
            &deploys,
            &mut pieces,
            &mut foe,
            0,
            0,
            CX + 2,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(
            last(&ev),
            (
                crate::world::EV_BUILD_REFUSED,
                8,
                crate::build::REFUSE_B_CLAIM,
                0
            )
        );
        place_deploy(
            SEED,
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut foe,
            0,
            3,
            CX + 2,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(last(&ev).2, REFUSE_D_CLAIM);
        // ~30 m away (10 cells): outside the 24 m radius.
        let far = CX + 10;
        let mut foe_far = player_at_cell(far, CZ, &[(0, 99), (1, 99), (5, 5)]);
        foe_far.id = 8;
        crate::build::place(
            SEED,
            &bc,
            &deploys,
            &mut pieces,
            &mut foe_far,
            0,
            0,
            far,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(last(&ev).0, crate::world::EV_PIECE_PLACED);

        // The owner may keep building inside their own radius…
        crate::build::place(
            SEED,
            &bc,
            &deploys,
            &mut pieces,
            &mut own,
            0,
            0,
            CX + 1,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(last(&ev).0, crate::world::EV_PIECE_PLACED);
        // …but not stack a second hearth inside the first's radius.
        place_deploy(
            SEED,
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut own,
            0,
            0,
            CX + 1,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(last(&ev).2, REFUSE_D_OVERLAP);
    }

    #[test]
    fn bag_cap_holds_per_owner() {
        let dc = DeployContent::probe_fixture();
        let bc = BuildContent::probe_fixture();
        let mut pieces = Pieces::new();
        let mut deploys = Deploys::new();
        let mut ev = EventQueue::default();
        let mut p = player_at_cell(CX, CZ, &[(5, 20)]);
        let mut placed = 0usize;
        // Cells along +x within reach of a moving player.
        for k in 0..BAG_CAP as u16 + 1 {
            p.body = Body::at(
                SEED,
                (CX + k) as f32 * crate::build::BUILD_CELL_M + 1.5,
                CZ as f32 * crate::build::BUILD_CELL_M + 1.5,
            );
            place_deploy(
                SEED,
                &dc,
                &bc,
                &mut pieces,
                &mut deploys,
                &mut p,
                0,
                3,
                CX + k,
                CZ,
                0,
                LOC_PLANE,
                &mut ev,
            );
            if last(&ev).0 == crate::world::EV_DEPLOY_PLACED {
                placed += 1;
            }
        }
        assert_eq!(placed, BAG_CAP);
        assert_eq!(last(&ev).2, REFUSE_D_BAG_CAP);
        // Every one of them places ready: the cap is how many answers a
        // player may stack, and a stack that had to warm up would make the
        // cap mean something else on the first death after a build.
        assert!(
            deploys.bag_ready().iter().all(|&r| r == 0),
            "a freshly placed bag must be ready for the next death"
        );
    }

    /// The knob is spoken in minutes (ALPHA §1 / DECISIONS §open, "bag
    /// cooldown · cap": 5 min) and the sim only owns ticks, so the literal
    /// the registry gate pins is checked against the arithmetic it stands
    /// for. Change `TICK_HZ` and this fires — which is the point: a
    /// cooldown that silently became eight minutes because the tick rate
    /// moved is a knob nobody spoke.
    #[test]
    fn the_cooldown_is_five_minutes_of_ticks() {
        assert_eq!(
            BAG_COOLDOWN_TICKS,
            5 * 60 * crate::limits::TICK_HZ as u64,
            "BAG_COOLDOWN_TICKS is no longer the 5 minutes DECISIONS.md §open declares"
        );
    }

    /// Three bags of one owner, and the scan takes the nearest to the body
    /// — then refuses to take it twice. Bag geometry, at the level the
    /// world cannot see: `World::respawn` only ever asks this one question
    /// and this is where the answer is made.
    #[test]
    fn the_nearest_ready_bag_answers_and_is_then_spent() {
        let dc = DeployContent::probe_fixture();
        let bc = BuildContent::probe_fixture();
        let mut pieces = Pieces::new();
        let mut deploys = Deploys::new();
        let mut ev = EventQueue::default();
        let mut p = player_at_cell(CX, CZ, &[(5, 20)]);
        // Three bags in a row along +x, placed near-then-far so "nearest"
        // cannot be satisfied by "first in the store".
        for k in [0u16, 2, 4] {
            p.body = Body::at(
                SEED,
                (CX + k) as f32 * crate::build::BUILD_CELL_M + 1.5,
                CZ as f32 * crate::build::BUILD_CELL_M + 1.5,
            );
            place_deploy(
                SEED,
                &dc,
                &bc,
                &mut pieces,
                &mut deploys,
                &mut p,
                0,
                3,
                CX + k,
                CZ,
                0,
                LOC_PLANE,
                &mut ev,
            );
            assert_eq!(last(&ev).0, crate::world::EV_DEPLOY_PLACED);
        }
        assert_eq!(deploys.len(), 3);
        let center = |cx: u16| cell_center(cx, CZ);

        // Die beside the far bag: the far bag answers, not the first one.
        let (fx, fz) = center(CX + 4);
        assert_eq!(
            deploys.claim_bag(&dc, 7, fx, fz, 100),
            Some((fx, fz)),
            "the scan did not take the bag nearest the body"
        );
        // …and it is spent for exactly the cooldown, so the same death
        // point now walks to the next-nearest instead.
        let (mx, mz) = center(CX + 2);
        assert_eq!(
            deploys.claim_bag(&dc, 7, fx, fz, 100),
            Some((mx, mz)),
            "a bag answered twice inside its own cooldown"
        );
        // One tick before it wakes it is still spent; on its own tick it
        // answers again. Off-by-one on a five-minute timer is the kind of
        // bug that only ever shows up in someone's raid.
        let ready = 100 + BAG_COOLDOWN_TICKS;
        assert_eq!(
            deploys.claim_bag(&dc, 7, fx, fz, ready - 1),
            Some(center(CX)),
            "the far bag woke early"
        );
        assert_eq!(
            deploys.claim_bag(&dc, 7, fx, fz, ready),
            Some((fx, fz)),
            "the far bag did not wake on the tick its cooldown named"
        );
    }

    /// Someone else's bag is not an answer, and neither is nothing. Both
    /// verdicts are `None`, which is what puts the body back on the ring.
    #[test]
    fn a_foreign_bag_is_not_your_bag() {
        let dc = DeployContent::probe_fixture();
        let bc = BuildContent::probe_fixture();
        let mut pieces = Pieces::new();
        let mut deploys = Deploys::new();
        let mut ev = EventQueue::default();
        let (x, z) = cell_center(CX, CZ);

        // Nothing placed at all.
        assert_eq!(deploys.claim_bag(&dc, 7, x, z, 0), None);

        let mut p = player_at_cell(CX, CZ, &[(5, 20)]);
        place_deploy(
            SEED,
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut p,
            0,
            3,
            CX,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(last(&ev).0, crate::world::EV_DEPLOY_PLACED);
        assert_eq!(
            deploys.claim_bag(&dc, 8, x, z, 0),
            None,
            "a stranger woke on someone else's bag — the owner check is the \
             whole of base ownership here"
        );
        // The refused scan must not have spent the owner's bag either.
        assert_eq!(deploys.claim_bag(&dc, 7, x, z, 0), Some((x, z)));
    }

    /// A workbench is not a bed. Every other archetype is invisible to the
    /// scan, so a base full of boxes still wakes you on the ring.
    #[test]
    fn only_a_bag_answers_a_death() {
        let dc = DeployContent::probe_fixture();
        let bc = BuildContent::probe_fixture();
        let mut pieces = Pieces::new();
        let mut deploys = Deploys::new();
        let mut ev = EventQueue::default();
        // Row 1 is the workbench (PLACE_ANY, so bare ground holds it).
        let mut p = player_at_cell(CX, CZ, &[(3, 5)]);
        place_deploy(
            SEED,
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut p,
            0,
            1,
            CX,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(last(&ev).0, crate::world::EV_DEPLOY_PLACED);
        let (x, z) = cell_center(CX, CZ);
        assert_eq!(
            deploys.claim_bag(&dc, 7, x, z, 0),
            None,
            "a workbench answered a death"
        );
    }

    #[test]
    fn feed_fills_stock_and_upkeep_charges_it() {
        let dc = DeployContent::probe_fixture();
        let bc = BuildContent::probe_fixture();
        let mut pieces = Pieces::new();
        let mut deploys = Deploys::new();
        let mut ev = EventQueue::default();
        let mut p = player_at_cell(CX, CZ, &[(0, 250), (1, 80), (2, 1)]);
        founded(&bc, &mut pieces, &mut p, CX, CZ);
        place_deploy(
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
        assert_eq!(last(&ev).0, crate::world::EV_DEPLOY_PLACED);

        // Feed: FEED_CHUNK of item 0 (has 245 left after the foundation's
        // 5), all 80 of item 1.
        feed(&dc, &mut deploys, &mut p, CX, CZ, 0, &mut ev);
        assert_eq!(last(&ev).0, crate::world::EV_STOCK);
        assert_eq!(deploys.hearths()[0].stock[0], FEED_CHUNK);
        assert_eq!(deploys.hearths()[0].stock[1], 80);
        // Feeding an address with no hearth refuses.
        feed(&dc, &mut deploys, &mut p, CX + 3, CZ, 0, &mut ev);
        assert_eq!(last(&ev).2, REFUSE_D_HEARTH);

        // Cross one upkeep period: the foundation (cost 5 × item 0 ⇒
        // charge ceil(5·10/2400) = 1) pays from stock; nothing decays.
        let mut pc = 0u32;
        let mut dcur = 0u32;
        let tick = UPKEEP_PERIOD_TICKS + 1;
        upkeep_sweep(
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            tick,
            &mut pc,
            &mut dcur,
            &mut tick_budget(),
            &mut ev,
        );
        assert_eq!(deploys.hearths()[0].stock[0], FEED_CHUNK - 1);
        assert_eq!(pieces.len(), 1);
        assert_eq!(
            pieces.entries()[0].hp,
            bc.pieces[0].hp,
            "paid pieces keep hp"
        );
    }

    #[test]
    fn unpaid_pieces_decay_away_and_take_their_deployable() {
        let dc = DeployContent::probe_fixture();
        let bc = BuildContent::probe_fixture();
        let mut pieces = Pieces::new();
        let mut deploys = Deploys::new();
        let mut ev = EventQueue::default();
        let mut p = player_at_cell(CX, CZ, &[(0, 99), (1, 99), (3, 5)]);
        founded(&bc, &mut pieces, &mut p, CX, CZ);
        // A workbench (any-class) on the foundation.
        place_deploy(
            SEED,
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut p,
            0,
            1,
            CX,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(last(&ev).0, crate::world::EV_DEPLOY_PLACED);

        // No hearth anywhere: the foundation decays 5 hp (5% of 100) per
        // period. 100 hp ⇒ gone by period 20; sweep with catch-up 4 needs
        // 5 passes over distinct hours.
        let mut pc = 0u32;
        let mut dcur = 0u32;
        for hour in 1..=20u64 {
            upkeep_sweep(
                &dc,
                &bc,
                &mut pieces,
                &mut deploys,
                hour * UPKEEP_PERIOD_TICKS + 1,
                &mut pc,
                &mut dcur,
                &mut tick_budget(),
                &mut ev,
            );
            if pieces.is_empty() {
                break;
            }
        }
        assert!(pieces.is_empty(), "unpaid foundation never decayed away");
        assert!(
            deploys.is_empty(),
            "the workbench on the decayed foundation must go with it"
        );
        let codes: Vec<u8> = ev.entries().iter().map(|e| e.code).collect();
        assert!(codes.contains(&crate::world::EV_PIECE_REMOVED));
        assert!(codes.contains(&crate::world::EV_DEPLOY_REMOVED));
    }

    #[test]
    fn covered_deployables_are_free_uncovered_ones_decay() {
        let dc = DeployContent::probe_fixture();
        let bc = BuildContent::probe_fixture();
        let mut pieces = Pieces::new();
        let mut deploys = Deploys::new();
        let mut ev = EventQueue::default();
        let mut p = player_at_cell(CX, CZ, &[(0, 250), (1, 99), (2, 1), (5, 5)]);
        founded(&bc, &mut pieces, &mut p, CX, CZ);
        place_deploy(
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
        feed(&dc, &mut deploys, &mut p, CX, CZ, 0, &mut ev);
        // A bag inside the radius and one far outside it.
        p.body = Body::at(
            SEED,
            (CX + 2) as f32 * crate::build::BUILD_CELL_M + 1.5,
            CZ as f32 * crate::build::BUILD_CELL_M + 1.5,
        );
        place_deploy(
            SEED,
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut p,
            0,
            3,
            CX + 2,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(last(&ev).0, crate::world::EV_DEPLOY_PLACED);
        let far = CX + 40;
        p.body = Body::at(
            SEED,
            far as f32 * crate::build::BUILD_CELL_M + 1.5,
            CZ as f32 * crate::build::BUILD_CELL_M + 1.5,
        );
        place_deploy(
            SEED,
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut p,
            0,
            3,
            far,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(last(&ev).0, crate::world::EV_DEPLOY_PLACED);

        let near_hp = deploys.find(CX + 2, CZ, 0, LOC_PLANE).unwrap().hp;
        let mut pc = 0u32;
        let mut dcur = 0u32;
        upkeep_sweep(
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            UPKEEP_PERIOD_TICKS + 1,
            &mut pc,
            &mut dcur,
            &mut tick_budget(),
            &mut ev,
        );
        assert_eq!(
            deploys.find(CX + 2, CZ, 0, LOC_PLANE).unwrap().hp,
            near_hp,
            "covered deployables pay nothing and lose nothing"
        );
        let far_rec = deploys.find(far, CZ, 0, LOC_PLANE).unwrap();
        assert!(
            far_rec.hp < dc.defs[3].hp,
            "uncovered deployables decay ({} vs {})",
            far_rec.hp,
            dc.defs[3].hp
        );
    }

    /// A westward strafe at yaw 0 (forward +Z, right +X — the collide.rs
    /// test convention).
    fn walk_west() -> crate::input::InputFrame {
        crate::input::InputFrame {
            seq: 1,
            move_x: -127,
            ..crate::input::InputFrame::default()
        }
    }

    /// Foundation + doorway (build row 3) + door (deploy row 2) on the
    /// west edge of (CX, CZ); returns the acting player.
    fn doored(
        bc: &BuildContent,
        dc: &DeployContent,
        pieces: &mut Pieces,
        deploys: &mut Deploys,
        ev: &mut EventQueue,
    ) -> Player {
        let mut p = player_at_cell(CX, CZ, &[(0, 99), (1, 99), (4, 2)]);
        founded(bc, pieces, &mut p, CX, CZ);
        crate::build::place(
            SEED, bc, deploys, pieces, &mut p, 0, 3, CX, CZ, 0, LOC_EDGE_W, ev,
        );
        assert_eq!(last(ev).0, crate::world::EV_PIECE_PLACED, "doorway lands");
        place_deploy(
            SEED, dc, bc, pieces, deploys, &mut p, 0, 2, CX, CZ, 0, LOC_EDGE_W, ev,
        );
        assert_eq!(last(ev).0, crate::world::EV_DEPLOY_PLACED, "door lands");
        p
    }

    /// Walk a fresh body west through the doorway; the x it pins at.
    fn walk_x_after(pieces: &Pieces) -> f32 {
        let mut b = crate::movement::Body::at(
            SEED,
            CX as f32 * crate::build::BUILD_CELL_M + 1.5,
            CZ as f32 * crate::build::BUILD_CELL_M + 1.5,
        );
        let f = walk_west();
        for _ in 0..120 {
            crate::movement::step(SEED, pieces.cols(), &mut b, &f);
        }
        b.qx as f32 * crate::movement::POS_XZ_Q
    }

    #[test]
    fn door_places_closed_toggles_open_and_reseals() {
        let dc = DeployContent::probe_fixture();
        let bc = BuildContent::probe_fixture();
        let mut pieces = Pieces::new();
        let mut deploys = Deploys::new();
        let mut ev = EventQueue::default();
        let mut p = doored(&bc, &dc, &mut pieces, &mut deploys, &mut ev);
        let wall_x = CX as f32 * crate::build::BUILD_CELL_M;

        // Placed closed: sim state, the shut bit, and the walk agree.
        assert!(!deploys.find(CX, CZ, 0, LOC_EDGE_W).unwrap().open);
        assert!(
            deploys.find(CX, CZ, 0, LOC_EDGE_W).unwrap().locked,
            "a door places locked to its placer (lock v0)"
        );
        assert_eq!(pieces.cols().get(CX, CZ).shut_w, 1);
        assert!(
            walk_x_after(&pieces) >= wall_x,
            "a closed door must block the doorway opening"
        );

        // Use opens: EV_DOOR announces, the same walk passes.
        use_door(
            &dc,
            &mut pieces,
            &mut deploys,
            &mut p,
            CX,
            CZ,
            0,
            LOC_EDGE_W,
            &mut ev,
        );
        assert_eq!(
            last(&ev),
            (
                crate::world::EV_DOOR,
                crate::gather::cell_key(CX, CZ),
                // open, and still locked — the announcement is the whole
                // door, absolute (lock v0).
                (LOC_EDGE_W as u32) << 8 | 2 | 1,
                7
            )
        );
        assert!(deploys.find(CX, CZ, 0, LOC_EDGE_W).unwrap().open);
        assert_eq!(pieces.cols().get(CX, CZ).shut_w, 0);
        assert!(
            walk_x_after(&pieces) < wall_x - 0.5,
            "an open door must pass like an empty doorway"
        );

        // Use again reseals.
        use_door(
            &dc,
            &mut pieces,
            &mut deploys,
            &mut p,
            CX,
            CZ,
            0,
            LOC_EDGE_W,
            &mut ev,
        );
        assert_eq!(last(&ev).2 & 1, 0, "EV_DOOR carries open = 0");
        assert_eq!(pieces.cols().get(CX, CZ).shut_w, 1);
        assert!(
            walk_x_after(&pieces) >= wall_x,
            "reclosed door blocks again"
        );
    }

    #[test]
    fn a_locked_door_answers_only_to_its_owner() {
        let dc = DeployContent::probe_fixture();
        let bc = BuildContent::probe_fixture();
        let mut pieces = Pieces::new();
        let mut deploys = Deploys::new();
        let mut ev = EventQueue::default();
        let mut p = doored(&bc, &dc, &mut pieces, &mut deploys, &mut ev);
        // A second hand at the same doorway, in reach, empty-handed.
        let mut stranger = player_at_cell(CX, CZ, &[]);
        stranger.id = 9;

        // Locked: the stranger bounces and the leaf does not move.
        use_door(
            &dc,
            &mut pieces,
            &mut deploys,
            &mut stranger,
            CX,
            CZ,
            0,
            LOC_EDGE_W,
            &mut ev,
        );
        assert_eq!(
            (last(&ev).0, last(&ev).2),
            (crate::world::EV_DEPLOY_REFUSED, REFUSE_D_OWNER),
            "a locked door refuses a foreign hand"
        );
        assert!(!deploys.find(CX, CZ, 0, LOC_EDGE_W).unwrap().open);
        assert_eq!(pieces.cols().get(CX, CZ).shut_w, 1, "and stays sealed");

        // The owner's own press still works, locked and all.
        use_door(
            &dc,
            &mut pieces,
            &mut deploys,
            &mut p,
            CX,
            CZ,
            0,
            LOC_EDGE_W,
            &mut ev,
        );
        assert!(deploys.find(CX, CZ, 0, LOC_EDGE_W).unwrap().open);

        // Unlocked, the door is public again — the door v0 behavior, now
        // something its owner chooses rather than the only option.
        set_lock(&dc, &mut deploys, &p, CX, CZ, 0, LOC_EDGE_W, false, &mut ev);
        assert_eq!(
            last(&ev),
            (
                crate::world::EV_DOOR,
                crate::gather::cell_key(CX, CZ),
                // Still open: unlocking never moves the leaf.
                (LOC_EDGE_W as u32) << 8 | 1,
                7
            )
        );
        use_door(
            &dc,
            &mut pieces,
            &mut deploys,
            &mut stranger,
            CX,
            CZ,
            0,
            LOC_EDGE_W,
            &mut ev,
        );
        assert_eq!(last(&ev).0, crate::world::EV_DOOR, "an unlocked door opens");
        assert!(!deploys.find(CX, CZ, 0, LOC_EDGE_W).unwrap().open);
        assert_eq!(pieces.cols().get(CX, CZ).shut_w, 1);

        // And a stranger may not lock what a stranger does not own.
        set_lock(
            &dc,
            &mut deploys,
            &stranger,
            CX,
            CZ,
            0,
            LOC_EDGE_W,
            true,
            &mut ev,
        );
        assert_eq!(
            (last(&ev).0, last(&ev).2),
            (crate::world::EV_DEPLOY_REFUSED, REFUSE_D_OWNER)
        );
        assert!(!deploys.find(CX, CZ, 0, LOC_EDGE_W).unwrap().locked);
    }

    #[test]
    fn lock_is_absolute_and_bounces_like_a_use() {
        let dc = DeployContent::probe_fixture();
        let bc = BuildContent::probe_fixture();
        let mut pieces = Pieces::new();
        let mut deploys = Deploys::new();
        let mut ev = EventQueue::default();
        let p = doored(&bc, &dc, &mut pieces, &mut deploys, &mut ev);

        // Absolute, not a toggle: the same request twice is the same
        // state twice (two presses racing must not fight).
        for _ in 0..2 {
            set_lock(&dc, &mut deploys, &p, CX, CZ, 0, LOC_EDGE_W, false, &mut ev);
            assert_eq!(last(&ev).0, crate::world::EV_DOOR);
            assert!(!deploys.find(CX, CZ, 0, LOC_EDGE_W).unwrap().locked);
        }
        set_lock(&dc, &mut deploys, &p, CX, CZ, 0, LOC_EDGE_W, true, &mut ev);
        assert!(deploys.find(CX, CZ, 0, LOC_EDGE_W).unwrap().locked);
        assert_eq!(
            last(&ev).2 & 3,
            2,
            "EV_DOOR carries locked = 1 over a closed leaf"
        );

        // The address must hold a door, and the hand must be in reach —
        // the same two refusals the use action bounces with.
        set_lock(&dc, &mut deploys, &p, CX, CZ, 0, LOC_PLANE, true, &mut ev);
        assert_eq!(last(&ev).2, REFUSE_D_DOOR, "no door at that address");
        let far = player_at_cell(CX + 7, CZ, &[]);
        set_lock(
            &dc,
            &mut deploys,
            &far,
            CX,
            CZ,
            0,
            LOC_EDGE_W,
            false,
            &mut ev,
        );
        assert_eq!(last(&ev).2, REFUSE_D_REACH, "a lock has the build reach");
        assert!(
            deploys.find(CX, CZ, 0, LOC_EDGE_W).unwrap().locked,
            "a refused lock leaves the bit where it was"
        );
    }

    #[test]
    fn use_refusals_name_no_door_and_reach() {
        let dc = DeployContent::probe_fixture();
        let bc = BuildContent::probe_fixture();
        let mut pieces = Pieces::new();
        let mut deploys = Deploys::new();
        let mut ev = EventQueue::default();

        // Nothing at the address.
        let mut p = player_at_cell(CX, CZ, &[(0, 99), (1, 99), (3, 1), (4, 2)]);
        use_door(
            &dc,
            &mut pieces,
            &mut deploys,
            &mut p,
            CX,
            CZ,
            0,
            LOC_EDGE_W,
            &mut ev,
        );
        assert_eq!(last(&ev).2, REFUSE_D_DOOR);

        // A deployable that is not a door (the any-class workbench).
        founded(&bc, &mut pieces, &mut p, CX, CZ);
        place_deploy(
            SEED,
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut p,
            0,
            1,
            CX,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(last(&ev).0, crate::world::EV_DEPLOY_PLACED);
        use_door(
            &dc,
            &mut pieces,
            &mut deploys,
            &mut p,
            CX,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(last(&ev).2, REFUSE_D_DOOR, "a workbench is not a door");

        // A real door, toggled from 20 m away: reach refusal, state kept.
        crate::build::place(
            SEED,
            &bc,
            &deploys,
            &mut pieces,
            &mut p,
            0,
            3,
            CX,
            CZ,
            0,
            LOC_EDGE_W,
            &mut ev,
        );
        place_deploy(
            SEED,
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut p,
            0,
            2,
            CX,
            CZ,
            0,
            LOC_EDGE_W,
            &mut ev,
        );
        assert_eq!(last(&ev).0, crate::world::EV_DEPLOY_PLACED);
        let mut far = player_at_cell(CX + 7, CZ, &[]);
        use_door(
            &dc,
            &mut pieces,
            &mut deploys,
            &mut far,
            CX,
            CZ,
            0,
            LOC_EDGE_W,
            &mut ev,
        );
        assert_eq!(last(&ev).2, REFUSE_D_REACH);
        assert!(!deploys.find(CX, CZ, 0, LOC_EDGE_W).unwrap().open);
        assert_eq!(pieces.cols().get(CX, CZ).shut_w, 1);
    }

    #[test]
    fn doorway_decay_takes_the_door_and_unseals() {
        let dc = DeployContent::probe_fixture();
        let bc = BuildContent::probe_fixture();
        let mut pieces = Pieces::new();
        let mut deploys = Deploys::new();
        let mut ev = EventQueue::default();
        doored(&bc, &dc, &mut pieces, &mut deploys, &mut ev);
        assert_eq!(pieces.cols().get(CX, CZ).shut_w, 1);

        // No hearth anywhere: everything decays away within 20 periods.
        let mut pc = 0u32;
        let mut dcur = 0u32;
        for hour in 1..=20u64 {
            upkeep_sweep(
                &dc,
                &bc,
                &mut pieces,
                &mut deploys,
                hour * UPKEEP_PERIOD_TICKS + 1,
                &mut pc,
                &mut dcur,
                &mut tick_budget(),
                &mut ev,
            );
            if pieces.is_empty() && deploys.is_empty() {
                break;
            }
        }
        assert!(pieces.is_empty(), "unpaid pieces never decayed away");
        assert!(
            deploys.is_empty(),
            "the door must go with (or before) its doorway"
        );
        let m = pieces.cols().get(CX, CZ);
        assert_eq!(
            (m.doors_w, m.shut_w),
            (0, 0),
            "a decayed doorway leaves no door collision behind"
        );
    }

    /// The box store's cap, and its stated overflow policy: **refuse**
    /// (wall 4). Written against the private `len` rather than by placing
    /// two hundred and fifty-six boxes, because the thing under test is
    /// the guard and not the terrain generator's supply of flat cells.
    ///
    /// The refusal has to land *before* the deploy record is inserted, or
    /// a full box store would leave a standing box with nowhere to keep
    /// what you put in it — a container that silently eats items, which is
    /// the one failure mode worse than refusing to place.
    #[test]
    fn a_full_box_store_refuses_the_placement_and_places_nothing() {
        let mut dc = DeployContent::probe_fixture();
        dc.defs[4] = DeployDef {
            arch: ARCH_BOX,
            placement: PLACE_FOUNDATION,
            hp: 100,
            item: 6,
        };
        dc.def_count = 5;
        let bc = BuildContent::probe_fixture();
        let mut pieces = Pieces::new();
        let mut deploys = Deploys::new();
        let mut ev = EventQueue::default();
        let mut p = player_at_cell(CX, CZ, &[(0, 9), (6, 4)]);
        founded(&bc, &mut pieces, &mut p, CX, CZ);

        deploys.boxes.len = MAX_BOXES;
        let deploys_before = deploys.len();
        let held = crate::craft::inv_count(&p.inv, 6);
        ev.clear();
        place_deploy(
            SEED,
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut p,
            0,
            4,
            CX,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(last(&ev).0, crate::world::EV_DEPLOY_REFUSED);
        assert_eq!(last(&ev).2, REFUSE_D_FULL, "a full box store refuses");
        assert_eq!(
            deploys.len(),
            deploys_before,
            "a refused box must not leave a deploy record standing"
        );
        assert_eq!(
            crate::craft::inv_count(&p.inv, 6),
            held,
            "a refused placement must not spend the item"
        );

        // One slot back and the same placement succeeds — a guard that
        // refused unconditionally would pass every assert above.
        deploys.boxes.len = MAX_BOXES - 1;
        ev.clear();
        place_deploy(
            SEED,
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut p,
            0,
            4,
            CX,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(last(&ev).0, crate::world::EV_DEPLOY_PLACED);
        assert_eq!(
            deploys.boxes.len, MAX_BOXES,
            "the record took the last slot"
        );
    }
}
