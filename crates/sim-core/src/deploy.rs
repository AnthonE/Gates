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
//!   **use** action toggles it open/closed: any player within build reach
//!   may toggle — no lock exists yet (locks are a later slice, the
//!   reference's unlocked-door behavior). EV_DOOR announces the new state
//!   for the wire; removal (decay, or the doorway decaying under it)
//!   clears the shut bit with the record. The state is absolute on the
//!   wire, never a delta, so the client that toggled optimistically
//!   (NETCODE.md §6.1) is confirmed or corrected by the same event.
//!
//! Not in this slice (documented, not forgotten): no support cascade when
//! a piece decays away (floating pieces keep decaying on their own if
//! unpaid), no door locks (any hand in reach toggles), no owner on the
//! wire (respawn-on-bag adds its own lane).

use crate::build::{BuildContent, Pieces, LOC_EDGE_N, LOC_EDGE_W, LOC_PLANE, SHAPE_DOORWAY};
use crate::craft::{inv_count, inv_take};
use crate::limits::{
    HEARTH_STOCK_ROWS, MAX_BUILD_COORD, MAX_BUILD_LEVELS, MAX_DEPLOYS, MAX_DEPLOY_DEFS,
    MAX_HEARTHS, UPKEEP_SWEEP_PER_TICK,
};
use crate::terrain;
use crate::world::{
    EventQueue, Player, EV_DEPLOY_PLACED, EV_DEPLOY_REFUSED, EV_DEPLOY_REMOVED, EV_DOOR,
    EV_PIECE_REMOVED, EV_STOCK,
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

/// The placed-deployable store: dense, insertion-ordered, plus the dense
/// hearth list the claim checks and the sweep scan. Removal swap-removes
/// (decay order is the sweep cursor's, deterministic); the wire layer
/// restarts in-progress sync walks on any removal.
pub struct Deploys {
    entries: [DeployRec; MAX_DEPLOYS],
    len: usize,
    hearths: [HearthRec; MAX_HEARTHS],
    hearth_count: usize,
}

impl Deploys {
    pub fn new() -> Self {
        Self {
            entries: [DeployRec::default(); MAX_DEPLOYS],
            len: 0,
            hearths: [HearthRec::default(); MAX_HEARTHS],
            hearth_count: 0,
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

    pub fn hearths(&self) -> &[HearthRec] {
        &self.hearths[..self.hearth_count]
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
        self.len += 1;
        true
    }

    /// Swap-remove entry `i`; drops the hearth record too when the entry
    /// was a hearth. The caller announces the removal event.
    fn remove_at(&mut self, i: usize, dc: &DeployContent) {
        let rec = self.entries[i];
        self.len -= 1;
        self.entries[i] = self.entries[self.len];
        if dc.defs[rec.row as usize].arch == ARCH_HEARTH {
            if let Some(h) = self.hearths[..self.hearth_count]
                .iter()
                .position(|h| h.cx == rec.cx && h.cz == rec.cz && h.level == rec.level)
            {
                self.hearth_count -= 1;
                self.hearths[h] = self.hearths[self.hearth_count];
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
}

impl Default for Deploys {
    fn default() -> Self {
        Self::new()
    }
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

/// Apply one use request (`Command::Use`): toggle the door at the
/// address. Any player within build reach may toggle — no lock exists
/// yet (door v0). The shut bit in the collision index flips in the same
/// call, so the tick that toggles is the tick that blocks (or opens);
/// EV_DOOR announces the new state for the wire.
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
    let Some(i) = deploys.entries[..deploys.len]
        .iter()
        .position(|d| d.cx == cx && d.cz == cz && d.level == level && d.loc == loc)
    else {
        events.push(EV_DEPLOY_REFUSED, p.id, REFUSE_D_DOOR, 0);
        return;
    };
    if dc.defs[deploys.entries[i].row as usize].arch != ARCH_DOOR {
        events.push(EV_DEPLOY_REFUSED, p.id, REFUSE_D_DOOR, 0);
        return;
    }
    let (ax, az) = cell_center(cx, cz);
    let (px, pz) = player_xz(p);
    let (dx, dz) = (ax - px, az - pz);
    if dx * dx + dz * dz > crate::build::BUILD_REACH_M * crate::build::BUILD_REACH_M {
        events.push(EV_DEPLOY_REFUSED, p.id, REFUSE_D_REACH, 0);
        return;
    }
    let open = !deploys.entries[i].open;
    deploys.entries[i].open = open;
    pieces.set_door(cx, cz, level, loc, !open);
    events.push(
        EV_DOOR,
        crate::gather::cell_key(cx, cz),
        ((level as u32) << 16) | ((loc as u32) << 8) | open as u32,
        p.id,
    );
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
            pieces.remove_at(i, def.shape);
            *piece_cursor = (i as u32).min(pieces.len().saturating_sub(1) as u32);
            events.push(
                EV_PIECE_REMOVED,
                crate::gather::cell_key(rec.cx, rec.cz),
                ((rec.level as u32) << 16) | ((rec.loc as u32) << 8) | rec.row as u32,
                0,
            );
            // The deployable standing on (or in) the removed piece.
            if let Some(di) = deploys.entries[..deploys.len].iter().position(|d| {
                d.cx == rec.cx && d.cz == rec.cz && d.level == rec.level && d.loc == rec.loc
            }) {
                let drec = deploys.entries[di];
                deploys.remove_at(di, dc);
                if dc.defs[drec.row as usize].arch == ARCH_DOOR {
                    // The door goes with its doorway; unseal the edge.
                    pieces.set_door(drec.cx, drec.cz, drec.level, drec.loc, false);
                }
                events.push(
                    EV_DEPLOY_REMOVED,
                    crate::gather::cell_key(drec.cx, drec.cz),
                    ((drec.level as u32) << 16) | ((drec.loc as u32) << 8) | drec.row as u32,
                    0,
                );
            }
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
            deploys.remove_at(i, dc);
            *deploy_cursor = (i as u32).min(deploys.len.saturating_sub(1) as u32);
            if def.arch == ARCH_DOOR {
                // A decayed door stops sealing its doorway.
                pieces.set_door(rec.cx, rec.cz, rec.level, rec.loc, false);
            }
            events.push(
                EV_DEPLOY_REMOVED,
                crate::gather::cell_key(rec.cx, rec.cz),
                ((rec.level as u32) << 16) | ((rec.loc as u32) << 8) | rec.row as u32,
                0,
            );
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
                (LOC_EDGE_W as u32) << 8 | 1,
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
}
