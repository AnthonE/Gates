//! The reliable event lane, S→C (NETCODE.md §2: the bidi stream carries
//! "chunk event fan-out · chat · transactions" — this module is that
//! lane's v1 schema, opened by the gather slice). One datagram-style kind
//! (`KIND_EVENT`) with a subtype field: gather payouts, own-inventory
//! updates, scatter-slot harvested/respawned facts, the batched
//! harvested-set join sync, and the item-name catalog.
//!
//! Everything here rides length-prefixed frames on the handshake bidi
//! stream (ordered + reliable), so there is no ack/baseline machinery:
//! a message arrives exactly once, in order, or the connection is dead.
//! Encode and decode are total and allocation-free like the datagrams —
//! the client decodes server bytes, the server encodes on the sim thread.

use crate::bits::{BitReader, BitWriter, WireError};
use crate::{
    expect_zero_padding, BUILD_CELL_BITS, BUILD_LEVEL_BITS, BUILD_LOC_BITS, DEPLOY_ROW_BITS,
    KIND_BITS, KIND_EVENT, PIECE_ROW_BITS,
};
use sim_core::build::{BuildContent, PieceDef, PieceRec, LOC_EDGE_N, MAT_METAL, SHAPE_ROOF};
use sim_core::craft::{CraftContent, CraftJob, RecipeDef, STATION_FURNACE};
use sim_core::deploy::{DeployContent, DeployDef, DeployRec, ARCH_DOOR, PLACE_ANY};
use sim_core::gather::ItemStack;
use sim_core::limits::{
    CRAFT_QUEUE, HEARTH_STOCK_ROWS, INV_SLOTS, MAX_BUILD_COORD, MAX_BUILD_LEVELS, MAX_DEPLOY_DEFS,
    MAX_ITEM_DEFS, MAX_PIECE_COSTS, MAX_PIECE_DEFS, MAX_RECIPES, MAX_RECIPE_INPUTS,
};

/// Longest event-lane message. Sized by the worst subtype (a full catalog
/// batch ≈ 264 B, a full slot-sync batch ≈ 258 B) with headroom; the
/// client-side framer refuses past it. Registered in DECISIONS.md §open.
pub const MAX_EVENT_MSG_BYTES: usize = 320;

/// Harvested cells one sync message carries (join sync is drip-fed, one
/// message per client per tick — server core). Overflow policy: the next
/// message continues the walk.
pub const SLOT_SYNC_BATCH: usize = 64;

/// Item names one catalog message carries.
pub const CATALOG_BATCH: usize = 8;

/// Recipe rows one recipes message carries (a full row is ~22 B; four
/// keep the drip well under the message cap).
pub const RECIPE_BATCH: usize = 4;

/// Placed-piece records one sync message carries (a record is 33 bits;
/// 32 keep the batch ≈ 134 B, well under the message cap). The join walk
/// is drip-fed like the harvested-set sync.
pub const PIECE_SYNC_BATCH: usize = 32;

/// Piece-def rows one defs message carries (a full row is ~11 B).
pub const PIECE_DEFS_BATCH: usize = 6;

/// Placed-deployable records one sync message carries (a record is 31
/// bits — address, row, open, locked; 24 keep the batch ≈ 94 B, well
/// under the message cap). The join walk is drip-fed like the piece sync.
pub const DEPLOY_SYNC_BATCH: usize = 24;

/// Deploy-def rows one defs message carries (a full row is ~5 B).
pub const DEPLOY_DEFS_BATCH: usize = 8;

/// Longest item display name on the wire; the catalog bake refuses past
/// it (content names are short by construction — CONTENT.md §2).
pub const MAX_ITEM_NAME_BYTES: usize = 24;

const SUB_BITS: u32 = 5;
const SUB_GATHER: u32 = 0;
const SUB_INV: u32 = 1;
const SUB_SLOT_HARVESTED: u32 = 2;
const SUB_SLOT_RESPAWNED: u32 = 3;
const SUB_SLOT_SYNC: u32 = 4;
const SUB_CATALOG: u32 = 5;
const SUB_WEAK_MARK: u32 = 6;
const SUB_CRAFT_Q: u32 = 7;
const SUB_CRAFT_DONE: u32 = 8;
const SUB_CRAFT_REFUSED: u32 = 9;
const SUB_RECIPES: u32 = 10;
const SUB_PIECE_PLACED: u32 = 11;
const SUB_PIECE_SYNC: u32 = 12;
const SUB_BUILD_REFUSED: u32 = 13;
const SUB_PIECE_DEFS: u32 = 14;
const SUB_DEPLOY_PLACED: u32 = 15;
const SUB_DEPLOY_SYNC: u32 = 16;
const SUB_DEPLOY_REFUSED: u32 = 17;
const SUB_DEPLOY_DEFS: u32 = 18;
const SUB_PIECE_REMOVED: u32 = 19;
const SUB_DEPLOY_REMOVED: u32 = 20;
const SUB_STOCK: u32 = 21;
const SUB_DOOR: u32 = 22;

const INV_COUNT_BITS: u32 = 5;
const INV_SLOT_BITS: u32 = 5;
const SYNC_COUNT_BITS: u32 = 7;
const CATALOG_TOTAL_BITS: u32 = 7;
const CATALOG_COUNT_BITS: u32 = 4;
const NAME_LEN_BITS: u32 = 5;
const CRAFT_Q_COUNT_BITS: u32 = 3;
const RECIPE_TOTAL_BITS: u32 = 7;
const RECIPE_COUNT_BITS: u32 = 3;
/// Craft time crosses as raw ticks (the value the sim runs), not seconds
/// — no cadence coupling; 24 bits cover the bake's ceiling (65535 s ×
/// TICK_HZ ≈ 2 M ticks) with headroom.
const RECIPE_TICKS_BITS: u32 = 24;
const STATION_BITS: u32 = 2;
const N_INPUTS_BITS: u32 = 3;
const PIECE_SYNC_COUNT_BITS: u32 = 6;
const PIECE_DEFS_TOTAL_BITS: u32 = 6;
const PIECE_DEFS_COUNT_BITS: u32 = 3;
const SHAPE_BITS: u32 = 3;
const MATERIAL_BITS: u32 = 2;
const N_COSTS_BITS: u32 = 2;
const DEPLOY_SYNC_COUNT_BITS: u32 = 5;
const DEPLOY_DEFS_TOTAL_BITS: u32 = 5;
const DEPLOY_DEFS_COUNT_BITS: u32 = 4;
const ARCH_BITS: u32 = 3;
const PLACEMENT_BITS: u32 = 2;
const STOCK_COUNT_BITS: u32 = 3;

/// One changed inventory slot on the wire.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InvSlot {
    pub slot: u8,
    pub stack: ItemStack,
}

/// The item-index → display-name table (the mapping `content::bake`
/// promises the wire ships). Server bakes it at boot; the client fills
/// its copy from catalog messages. Fixed storage, `MAX_ITEM_DEFS` rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ItemCatalog {
    pub names: [[u8; MAX_ITEM_NAME_BYTES]; MAX_ITEM_DEFS],
    pub lens: [u8; MAX_ITEM_DEFS],
    pub count: u16,
}

impl ItemCatalog {
    pub const EMPTY: Self = Self {
        names: [[0; MAX_ITEM_NAME_BYTES]; MAX_ITEM_DEFS],
        lens: [0; MAX_ITEM_DEFS],
        count: 0,
    };

    /// Install one name. Refuses empty, oversize, or out-of-table — the
    /// server's bake path turns this into a refused boot.
    pub fn set(&mut self, idx: usize, name: &[u8]) -> Result<(), WireError> {
        if idx >= MAX_ITEM_DEFS || name.is_empty() || name.len() > MAX_ITEM_NAME_BYTES {
            return Err(WireError::Range);
        }
        self.names[idx][..name.len()].copy_from_slice(name);
        self.names[idx][name.len()..].fill(0);
        self.lens[idx] = name.len() as u8;
        Ok(())
    }

    pub fn name(&self, idx: usize) -> &[u8] {
        if idx < MAX_ITEM_DEFS {
            &self.names[idx][..self.lens[idx] as usize]
        } else {
            &[]
        }
    }
}

impl Default for ItemCatalog {
    fn default() -> Self {
        Self::EMPTY
    }
}

/// One decoded event-lane message. Fixed storage; unused tails stay zero
/// so equality is well-defined (the goldens compare decoded values).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventMsg {
    /// Own gather payout: `added` units of `item` landed (or 0 — full
    /// inventory). The toast, not the truth: `Inv` is authoritative.
    Gather { item: u16, added: u16 },
    /// Authoritative own-inventory slots that changed since last sent.
    Inv {
        slots: [InvSlot; INV_SLOTS],
        count: u8,
    },
    /// A scatter slot was exhausted — the node vanishes until respawn.
    SlotHarvested { cx: u16, cz: u16 },
    /// A harvested slot's timer arrived — the node stands again.
    SlotRespawned { cx: u16, cz: u16 },
    /// One batch of the harvested-cell walk. `reset` (first batch of a
    /// join or an event-lane resync) clears the client's set first.
    SlotSync {
        reset: bool,
        cells: [(u16, u16); SLOT_SYNC_BATCH],
        count: u8,
    },
    /// Item display names `first..first+count` of a `total`-row table.
    Catalog {
        total: u8,
        first: u8,
        count: u8,
        names: [[u8; MAX_ITEM_NAME_BYTES]; CATALOG_BATCH],
        lens: [u8; CATALOG_BATCH],
    },
    /// Own weak-spot mark after a landed hit (swinger-only): the node's
    /// cell, the next mark heading (u8 over the shared 256-entry yaw LUT,
    /// pointing node → stand point), and whether that hit was a weak hit.
    WeakMark {
        cx: u16,
        cz: u16,
        mark8: u8,
        weak_hit: bool,
    },
    /// Authoritative own craft queue after a change: `count` live jobs
    /// (dense, head first) as (recipe index, units remaining), plus the
    /// head unit's remaining ticks at send time (the client counts down
    /// locally between messages).
    CraftQ {
        jobs: [(u8, u8); CRAFT_QUEUE],
        count: u8,
        eta_ticks: u16,
    },
    /// One craft unit completed: `added` units of `item` landed (0 = full
    /// inventory — the loss is announced). The toast; `Inv` is the truth.
    CraftDone { item: u16, added: u16 },
    /// A craft request bounced: `reason` is a `sim_core::craft::REFUSE_*`
    /// code (unknown values render as a generic refusal).
    CraftRefused { reason: u8 },
    /// Recipe rows `first..first+count` of a `total`-row table — the
    /// craft menu's data, dripped like the item catalog. Rows decode to
    /// the same `RecipeDef` the sim runs (craft time crosses as ticks).
    Recipes {
        total: u8,
        first: u8,
        count: u8,
        rows: [RecipeDef; RECIPE_BATCH],
    },
    /// A building piece landed (broadcast — pieces are world facts like
    /// slot changes). The record is the sim's own `PieceRec`.
    PiecePlaced { rec: PieceRec },
    /// One batch of the placed-piece walk (join sync / event-lane resync).
    /// `reset` clears the client's piece set first.
    PieceSync {
        reset: bool,
        recs: [PieceRec; PIECE_SYNC_BATCH],
        count: u8,
    },
    /// A place request bounced: `reason` is a `sim_core::build::REFUSE_B_*`
    /// code (unknown values render as a generic refusal).
    BuildRefused { reason: u8 },
    /// Piece-def rows `first..first+count` of a `total`-row table — the
    /// build menu's data, dripped like the recipe table. Rows decode to
    /// the same `PieceDef` the sim runs.
    PieceDefs {
        total: u8,
        first: u8,
        count: u8,
        rows: [PieceDef; PIECE_DEFS_BATCH],
    },
    /// A deployable landed (broadcast — world facts like pieces). The
    /// wire carries address + row + the door's open bit; owner/hp/uh stay
    /// sim-side, so decoded records hold their defaults there.
    DeployPlaced { rec: DeployRec },
    /// One batch of the placed-deployable walk (join sync / resync).
    DeploySync {
        reset: bool,
        recs: [DeployRec; DEPLOY_SYNC_BATCH],
        count: u8,
    },
    /// A deploy or feed request bounced: `reason` is a
    /// `sim_core::deploy::REFUSE_D_*` code.
    DeployRefused { reason: u8 },
    /// Deploy-def rows `first..first+count` of a `total`-row table — the
    /// deployable menu's data, dripped like the piece defs.
    DeployDefs {
        total: u8,
        first: u8,
        count: u8,
        rows: [DeployDef; DEPLOY_DEFS_BATCH],
    },
    /// Decay removed the piece at the address (broadcast; in-progress
    /// piece walks restart server-side).
    PieceRemoved {
        cx: u16,
        cz: u16,
        level: u8,
        loc: u8,
    },
    /// Decay (or its supporting piece's removal) removed the deployable
    /// at the address.
    DeployRemoved {
        cx: u16,
        cz: u16,
        level: u8,
        loc: u8,
    },
    /// The door at the address changed state (broadcast — a door is a
    /// world fact like the piece it sits in). `open` and `locked` are the
    /// state after the action, never a delta: a client that missed one
    /// hears the truth from the next one. Which hand may open a locked
    /// door is not on the wire — the owner id stays sim-side, so a client
    /// presses and learns from the outcome (lock v0).
    Door {
        cx: u16,
        cz: u16,
        level: u8,
        loc: u8,
        open: bool,
        locked: bool,
    },
    /// The feed ack: the hearth's stock rows after the transfer, aligned
    /// to the baked upkeep-material list (item index, units).
    Stock {
        cx: u16,
        cz: u16,
        level: u8,
        rows: [(u16, u32); HEARTH_STOCK_ROWS],
        count: u8,
    },
}

fn begin(buf: &mut [u8], subtype: u32) -> Result<BitWriter<'_>, WireError> {
    let mut w = BitWriter::new(buf);
    w.write(KIND_EVENT, KIND_BITS)?;
    w.write(subtype, SUB_BITS)?;
    Ok(w)
}

pub fn encode_event_gather(item: u16, added: u16, buf: &mut [u8]) -> Result<usize, WireError> {
    let mut w = begin(buf, SUB_GATHER)?;
    w.write(item as u32, 16)?;
    w.write(added as u32, 16)?;
    Ok(w.finish())
}

/// `slots` must be non-empty and within `INV_SLOTS` rows/indices — an
/// empty update is a server bug, not a message.
pub fn encode_event_inv(slots: &[InvSlot], buf: &mut [u8]) -> Result<usize, WireError> {
    if slots.is_empty() || slots.len() > INV_SLOTS {
        return Err(WireError::Cap);
    }
    let mut w = begin(buf, SUB_INV)?;
    w.write(slots.len() as u32, INV_COUNT_BITS)?;
    for s in slots {
        if s.slot as usize >= INV_SLOTS {
            return Err(WireError::Range);
        }
        w.write(s.slot as u32, INV_SLOT_BITS)?;
        w.write(s.stack.item as u32, 16)?;
        w.write(s.stack.count as u32, 16)?;
    }
    Ok(w.finish())
}

pub fn encode_event_slot_change(
    harvested: bool,
    cx: u16,
    cz: u16,
    buf: &mut [u8],
) -> Result<usize, WireError> {
    let sub = if harvested {
        SUB_SLOT_HARVESTED
    } else {
        SUB_SLOT_RESPAWNED
    };
    let mut w = begin(buf, sub)?;
    w.write(cx as u32, 16)?;
    w.write(cz as u32, 16)?;
    Ok(w.finish())
}

/// A batch may be empty only with `reset` — the "your set is now empty"
/// resync message; an empty non-reset batch says nothing.
pub fn encode_event_slot_sync(
    reset: bool,
    cells: &[(u16, u16)],
    buf: &mut [u8],
) -> Result<usize, WireError> {
    if cells.len() > SLOT_SYNC_BATCH || (cells.is_empty() && !reset) {
        return Err(WireError::Cap);
    }
    let mut w = begin(buf, SUB_SLOT_SYNC)?;
    w.write_bit(reset)?;
    w.write(cells.len() as u32, SYNC_COUNT_BITS)?;
    for &(cx, cz) in cells {
        w.write(cx as u32, 16)?;
        w.write(cz as u32, 16)?;
    }
    Ok(w.finish())
}

/// Encode up to `CATALOG_BATCH` names starting at `first`. Returns the
/// byte length and how many names rode along; the caller's cursor
/// advances by that count.
pub fn encode_event_catalog(
    catalog: &ItemCatalog,
    first: usize,
    buf: &mut [u8],
) -> Result<(usize, usize), WireError> {
    let total = catalog.count as usize;
    if total > MAX_ITEM_DEFS || first >= total {
        return Err(WireError::Range);
    }
    let count = CATALOG_BATCH.min(total - first);
    let mut w = begin(buf, SUB_CATALOG)?;
    w.write(total as u32, CATALOG_TOTAL_BITS)?;
    w.write(first as u32, CATALOG_TOTAL_BITS)?;
    w.write(count as u32, CATALOG_COUNT_BITS)?;
    for idx in first..first + count {
        let name = catalog.name(idx);
        if name.is_empty() || name.len() > MAX_ITEM_NAME_BYTES {
            return Err(WireError::Range);
        }
        w.write(name.len() as u32, NAME_LEN_BITS)?;
        for &b in name {
            w.write(b as u32, 8)?;
        }
    }
    Ok((w.finish(), count))
}

pub fn encode_event_weak_mark(
    cx: u16,
    cz: u16,
    mark8: u8,
    weak_hit: bool,
    buf: &mut [u8],
) -> Result<usize, WireError> {
    let mut w = begin(buf, SUB_WEAK_MARK)?;
    w.write(cx as u32, 16)?;
    w.write(cz as u32, 16)?;
    w.write(mark8 as u32, 8)?;
    w.write_bit(weak_hit)?;
    Ok(w.finish())
}

/// `jobs` is the dense live prefix of a player's queue (empty ⇒ "queue
/// cleared"). Refuses a job the wire's widths can't carry: a dead job
/// (`remaining == 0`), a remaining over u8, or a recipe outside the table.
pub fn encode_event_craft_q(
    jobs: &[CraftJob],
    eta_ticks: u16,
    buf: &mut [u8],
) -> Result<usize, WireError> {
    if jobs.len() > CRAFT_QUEUE {
        return Err(WireError::Cap);
    }
    let mut w = begin(buf, SUB_CRAFT_Q)?;
    w.write(jobs.len() as u32, CRAFT_Q_COUNT_BITS)?;
    for j in jobs {
        if j.remaining == 0 || j.remaining > u8::MAX as u16 || j.recipe as usize >= MAX_RECIPES {
            return Err(WireError::Range);
        }
        w.write(j.recipe as u32, 8)?;
        w.write(j.remaining as u32, 8)?;
    }
    w.write(eta_ticks as u32, 16)?;
    Ok(w.finish())
}

pub fn encode_event_craft_done(item: u16, added: u16, buf: &mut [u8]) -> Result<usize, WireError> {
    let mut w = begin(buf, SUB_CRAFT_DONE)?;
    w.write(item as u32, 16)?;
    w.write(added as u32, 16)?;
    Ok(w.finish())
}

pub fn encode_event_craft_refused(reason: u8, buf: &mut [u8]) -> Result<usize, WireError> {
    let mut w = begin(buf, SUB_CRAFT_REFUSED)?;
    w.write(reason as u32, 8)?;
    Ok(w.finish())
}

/// Encode up to `RECIPE_BATCH` baked recipe rows starting at `first`.
/// Returns the byte length and how many rows rode along. Row shapes the
/// bake refuses (zero ticks, zero output, too many inputs) refuse here
/// too — this encoder only ever sees baked tables.
pub fn encode_event_recipes(
    cc: &CraftContent,
    first: usize,
    buf: &mut [u8],
) -> Result<(usize, usize), WireError> {
    let total = cc.recipe_count as usize;
    if total > MAX_RECIPES || first >= total {
        return Err(WireError::Range);
    }
    let count = RECIPE_BATCH.min(total - first);
    let mut w = begin(buf, SUB_RECIPES)?;
    w.write(total as u32, RECIPE_TOTAL_BITS)?;
    w.write(first as u32, RECIPE_TOTAL_BITS)?;
    w.write(count as u32, RECIPE_COUNT_BITS)?;
    for def in cc.recipes[first..first + count].iter() {
        if def.ticks == 0
            || def.ticks >= (1 << RECIPE_TICKS_BITS)
            || def.out_count == 0
            || def.out_count > u8::MAX as u16
            || def.station > STATION_FURNACE
            || def.n_inputs == 0
            || def.n_inputs as usize > MAX_RECIPE_INPUTS
        {
            return Err(WireError::Range);
        }
        w.write(def.output as u32, 16)?;
        w.write(def.out_count as u32, 8)?;
        w.write(def.ticks, RECIPE_TICKS_BITS)?;
        w.write(def.station as u32, STATION_BITS)?;
        w.write(def.n_inputs as u32, N_INPUTS_BITS)?;
        for &(item, per) in def.inputs.iter().take(def.n_inputs as usize) {
            w.write(item as u32, 16)?;
            w.write(per as u32, 16)?;
        }
    }
    Ok((w.finish(), count))
}

/// One placed-piece record on the wire: 33 bits, shared by the placed
/// broadcast and the sync batches. Refuses an address outside the grid or
/// a row outside the def table — this encoder only ever sees sim records.
fn write_piece_rec(w: &mut BitWriter, rec: &PieceRec) -> Result<(), WireError> {
    if rec.cx as usize >= MAX_BUILD_COORD
        || rec.cz as usize >= MAX_BUILD_COORD
        || rec.level as usize >= MAX_BUILD_LEVELS
        || rec.loc > LOC_EDGE_N
        || rec.row as usize >= MAX_PIECE_DEFS
    {
        return Err(WireError::Range);
    }
    w.write(rec.cx as u32, BUILD_CELL_BITS)?;
    w.write(rec.cz as u32, BUILD_CELL_BITS)?;
    w.write(rec.level as u32, BUILD_LEVEL_BITS)?;
    w.write(rec.loc as u32, BUILD_LOC_BITS)?;
    w.write(rec.row as u32, PIECE_ROW_BITS)?;
    Ok(())
}

fn read_piece_rec(r: &mut BitReader) -> Result<PieceRec, WireError> {
    let rec = PieceRec {
        cx: r.read(BUILD_CELL_BITS)? as u16,
        cz: r.read(BUILD_CELL_BITS)? as u16,
        level: r.read(BUILD_LEVEL_BITS)? as u8,
        loc: r.read(BUILD_LOC_BITS)? as u8,
        row: r.read(PIECE_ROW_BITS)? as u8,
        ..PieceRec::default()
    };
    // Coord/level/loc widths are exact; only the row can be forged.
    if rec.row as usize >= MAX_PIECE_DEFS {
        return Err(WireError::Malformed);
    }
    Ok(rec)
}

pub fn encode_event_piece_placed(rec: &PieceRec, buf: &mut [u8]) -> Result<usize, WireError> {
    let mut w = begin(buf, SUB_PIECE_PLACED)?;
    write_piece_rec(&mut w, rec)?;
    Ok(w.finish())
}

/// A batch may be empty only with `reset` — the "your piece set is now
/// empty" resync message, same contract as the slot sync.
pub fn encode_event_piece_sync(
    reset: bool,
    recs: &[PieceRec],
    buf: &mut [u8],
) -> Result<usize, WireError> {
    if recs.len() > PIECE_SYNC_BATCH || (recs.is_empty() && !reset) {
        return Err(WireError::Cap);
    }
    let mut w = begin(buf, SUB_PIECE_SYNC)?;
    w.write_bit(reset)?;
    w.write(recs.len() as u32, PIECE_SYNC_COUNT_BITS)?;
    for rec in recs {
        write_piece_rec(&mut w, rec)?;
    }
    Ok(w.finish())
}

pub fn encode_event_build_refused(reason: u8, buf: &mut [u8]) -> Result<usize, WireError> {
    let mut w = begin(buf, SUB_BUILD_REFUSED)?;
    w.write(reason as u32, 8)?;
    Ok(w.finish())
}

/// Encode up to `PIECE_DEFS_BATCH` baked piece rows starting at `first`.
/// Returns the byte length and how many rows rode along. Row shapes the
/// bake refuses (hp 0, too many costs) refuse here too.
pub fn encode_event_piece_defs(
    bc: &BuildContent,
    first: usize,
    buf: &mut [u8],
) -> Result<(usize, usize), WireError> {
    let total = bc.piece_count as usize;
    if total > MAX_PIECE_DEFS || first >= total {
        return Err(WireError::Range);
    }
    let count = PIECE_DEFS_BATCH.min(total - first);
    let mut w = begin(buf, SUB_PIECE_DEFS)?;
    w.write(total as u32, PIECE_DEFS_TOTAL_BITS)?;
    w.write(first as u32, PIECE_DEFS_TOTAL_BITS)?;
    w.write(count as u32, PIECE_DEFS_COUNT_BITS)?;
    for def in bc.pieces[first..first + count].iter() {
        if def.shape > SHAPE_ROOF
            || def.material > MAT_METAL
            || def.hp == 0
            || def.n_costs == 0
            || def.n_costs as usize > MAX_PIECE_COSTS
        {
            return Err(WireError::Range);
        }
        w.write(def.shape as u32, SHAPE_BITS)?;
        w.write(def.material as u32, MATERIAL_BITS)?;
        w.write(def.hp as u32, 16)?;
        w.write(def.n_costs as u32, N_COSTS_BITS)?;
        for &(item, units) in def.costs.iter().take(def.n_costs as usize) {
            w.write(item as u32, 16)?;
            w.write(units as u32, 16)?;
        }
    }
    Ok((w.finish(), count))
}

/// One placed-deployable record on the wire: 31 bits, shared by the
/// placed broadcast and the sync batches. Every width is exact, so only
/// sim-impossible addresses need refusing at encode. The trailing two
/// bits are the door's open and locked state — meaningless for every
/// other archetype, and always 0 there, so they cost two bits and save a
/// second lane for the join walk (a client that walked in must see which
/// doors stand open, and which stand locked).
fn write_deploy_rec(w: &mut BitWriter, rec: &DeployRec) -> Result<(), WireError> {
    if rec.cx as usize >= MAX_BUILD_COORD
        || rec.cz as usize >= MAX_BUILD_COORD
        || rec.level as usize >= MAX_BUILD_LEVELS
        || rec.loc > LOC_EDGE_N
        || rec.row as usize >= MAX_DEPLOY_DEFS
    {
        return Err(WireError::Range);
    }
    w.write(rec.cx as u32, BUILD_CELL_BITS)?;
    w.write(rec.cz as u32, BUILD_CELL_BITS)?;
    w.write(rec.level as u32, BUILD_LEVEL_BITS)?;
    w.write(rec.loc as u32, BUILD_LOC_BITS)?;
    w.write(rec.row as u32, DEPLOY_ROW_BITS)?;
    w.write_bit(rec.open)?;
    w.write_bit(rec.locked)?;
    Ok(())
}

fn read_deploy_rec(r: &mut BitReader) -> Result<DeployRec, WireError> {
    Ok(DeployRec {
        cx: r.read(BUILD_CELL_BITS)? as u16,
        cz: r.read(BUILD_CELL_BITS)? as u16,
        level: r.read(BUILD_LEVEL_BITS)? as u8,
        loc: r.read(BUILD_LOC_BITS)? as u8,
        row: r.read(DEPLOY_ROW_BITS)? as u8,
        open: r.read_bit()?,
        locked: r.read_bit()?,
        ..DeployRec::default()
    })
}

pub fn encode_event_deploy_placed(rec: &DeployRec, buf: &mut [u8]) -> Result<usize, WireError> {
    let mut w = begin(buf, SUB_DEPLOY_PLACED)?;
    write_deploy_rec(&mut w, rec)?;
    Ok(w.finish())
}

/// A batch may be empty only with `reset` — the same contract as the
/// piece sync.
pub fn encode_event_deploy_sync(
    reset: bool,
    recs: &[DeployRec],
    buf: &mut [u8],
) -> Result<usize, WireError> {
    if recs.len() > DEPLOY_SYNC_BATCH || (recs.is_empty() && !reset) {
        return Err(WireError::Cap);
    }
    let mut w = begin(buf, SUB_DEPLOY_SYNC)?;
    w.write_bit(reset)?;
    w.write(recs.len() as u32, DEPLOY_SYNC_COUNT_BITS)?;
    for rec in recs {
        write_deploy_rec(&mut w, rec)?;
    }
    Ok(w.finish())
}

pub fn encode_event_deploy_refused(reason: u8, buf: &mut [u8]) -> Result<usize, WireError> {
    let mut w = begin(buf, SUB_DEPLOY_REFUSED)?;
    w.write(reason as u32, 8)?;
    Ok(w.finish())
}

/// Encode up to `DEPLOY_DEFS_BATCH` baked deployable rows starting at
/// `first`. Returns the byte length and how many rows rode along. Row
/// shapes the bake refuses (hp 0, out-of-range codes) refuse here too.
pub fn encode_event_deploy_defs(
    dc: &DeployContent,
    first: usize,
    buf: &mut [u8],
) -> Result<(usize, usize), WireError> {
    let total = dc.def_count as usize;
    if total > MAX_DEPLOY_DEFS || first >= total {
        return Err(WireError::Range);
    }
    let count = DEPLOY_DEFS_BATCH.min(total - first);
    let mut w = begin(buf, SUB_DEPLOY_DEFS)?;
    w.write(total as u32, DEPLOY_DEFS_TOTAL_BITS)?;
    w.write(first as u32, DEPLOY_DEFS_TOTAL_BITS)?;
    w.write(count as u32, DEPLOY_DEFS_COUNT_BITS)?;
    for def in dc.defs[first..first + count].iter() {
        if def.arch > ARCH_DOOR || def.placement > PLACE_ANY || def.hp == 0 {
            return Err(WireError::Range);
        }
        w.write(def.arch as u32, ARCH_BITS)?;
        w.write(def.placement as u32, PLACEMENT_BITS)?;
        w.write(def.hp as u32, 16)?;
        w.write(def.item as u32, 16)?;
    }
    Ok((w.finish(), count))
}

/// A decay removal at a grid address — `piece` picks which store.
pub fn encode_event_removed(
    piece: bool,
    cx: u16,
    cz: u16,
    level: u8,
    loc: u8,
    buf: &mut [u8],
) -> Result<usize, WireError> {
    if cx as usize >= MAX_BUILD_COORD
        || cz as usize >= MAX_BUILD_COORD
        || level as usize >= MAX_BUILD_LEVELS
        || loc > LOC_EDGE_N
    {
        return Err(WireError::Range);
    }
    let sub = if piece {
        SUB_PIECE_REMOVED
    } else {
        SUB_DEPLOY_REMOVED
    };
    let mut w = begin(buf, sub)?;
    w.write(cx as u32, BUILD_CELL_BITS)?;
    w.write(cz as u32, BUILD_CELL_BITS)?;
    w.write(level as u32, BUILD_LEVEL_BITS)?;
    w.write(loc as u32, BUILD_LOC_BITS)?;
    Ok(w.finish())
}

/// The feed ack: `rows` are the hearth's live stock rows, aligned to the
/// baked upkeep-material list. Empty is legal (a hearth with no priced
/// materials cannot exist, but the width allows the message shape).
pub fn encode_event_stock(
    cx: u16,
    cz: u16,
    level: u8,
    rows: &[(u16, u32)],
    buf: &mut [u8],
) -> Result<usize, WireError> {
    if rows.len() > HEARTH_STOCK_ROWS {
        return Err(WireError::Cap);
    }
    if cx as usize >= MAX_BUILD_COORD
        || cz as usize >= MAX_BUILD_COORD
        || level as usize >= MAX_BUILD_LEVELS
    {
        return Err(WireError::Range);
    }
    let mut w = begin(buf, SUB_STOCK)?;
    w.write(cx as u32, BUILD_CELL_BITS)?;
    w.write(cz as u32, BUILD_CELL_BITS)?;
    w.write(level as u32, BUILD_LEVEL_BITS)?;
    w.write(rows.len() as u32, STOCK_COUNT_BITS)?;
    for &(item, units) in rows {
        w.write(item as u32, 16)?;
        w.write(units, 32)?;
    }
    Ok(w.finish())
}

/// The door at the address is now `open` and `locked` (broadcast).
/// Absolute state, not a toggle: two of these crossing never leave a
/// client inverted, and one lane carries the whole door so a client never
/// holds half of it.
pub fn encode_event_door(
    cx: u16,
    cz: u16,
    level: u8,
    loc: u8,
    open: bool,
    locked: bool,
    buf: &mut [u8],
) -> Result<usize, WireError> {
    if cx as usize >= MAX_BUILD_COORD
        || cz as usize >= MAX_BUILD_COORD
        || level as usize >= MAX_BUILD_LEVELS
        || loc > LOC_EDGE_N
    {
        return Err(WireError::Range);
    }
    let mut w = begin(buf, SUB_DOOR)?;
    w.write(cx as u32, BUILD_CELL_BITS)?;
    w.write(cz as u32, BUILD_CELL_BITS)?;
    w.write(level as u32, BUILD_LEVEL_BITS)?;
    w.write(loc as u32, BUILD_LOC_BITS)?;
    w.write_bit(open)?;
    w.write_bit(locked)?;
    Ok(w.finish())
}

/// Total decode of one event-lane message: arbitrary bytes in, `Ok` or a
/// `WireError` out, never a panic — same contract as the datagrams.
pub fn decode_event(buf: &[u8]) -> Result<EventMsg, WireError> {
    let mut r = BitReader::new(buf);
    if r.read(KIND_BITS)? != KIND_EVENT {
        return Err(WireError::Malformed);
    }
    let msg = match r.read(SUB_BITS)? {
        SUB_GATHER => EventMsg::Gather {
            item: r.read(16)? as u16,
            added: r.read(16)? as u16,
        },
        SUB_INV => {
            let count = r.read(INV_COUNT_BITS)? as usize;
            if count == 0 || count > INV_SLOTS {
                return Err(WireError::Malformed);
            }
            let mut slots = [InvSlot::default(); INV_SLOTS];
            for s in slots.iter_mut().take(count) {
                let slot = r.read(INV_SLOT_BITS)? as u8;
                if slot as usize >= INV_SLOTS {
                    return Err(WireError::Malformed);
                }
                *s = InvSlot {
                    slot,
                    stack: ItemStack {
                        item: r.read(16)? as u16,
                        count: r.read(16)? as u16,
                    },
                };
            }
            EventMsg::Inv {
                slots,
                count: count as u8,
            }
        }
        sub @ (SUB_SLOT_HARVESTED | SUB_SLOT_RESPAWNED) => {
            let cx = r.read(16)? as u16;
            let cz = r.read(16)? as u16;
            if sub == SUB_SLOT_HARVESTED {
                EventMsg::SlotHarvested { cx, cz }
            } else {
                EventMsg::SlotRespawned { cx, cz }
            }
        }
        SUB_SLOT_SYNC => {
            let reset = r.read_bit()?;
            let count = r.read(SYNC_COUNT_BITS)? as usize;
            if count > SLOT_SYNC_BATCH || (count == 0 && !reset) {
                return Err(WireError::Malformed);
            }
            let mut cells = [(0u16, 0u16); SLOT_SYNC_BATCH];
            for c in cells.iter_mut().take(count) {
                *c = (r.read(16)? as u16, r.read(16)? as u16);
            }
            EventMsg::SlotSync {
                reset,
                cells,
                count: count as u8,
            }
        }
        SUB_CATALOG => {
            let total = r.read(CATALOG_TOTAL_BITS)? as usize;
            let first = r.read(CATALOG_TOTAL_BITS)? as usize;
            let count = r.read(CATALOG_COUNT_BITS)? as usize;
            if total > MAX_ITEM_DEFS || count == 0 || count > CATALOG_BATCH || first + count > total
            {
                return Err(WireError::Malformed);
            }
            let mut names = [[0u8; MAX_ITEM_NAME_BYTES]; CATALOG_BATCH];
            let mut lens = [0u8; CATALOG_BATCH];
            for i in 0..count {
                let len = r.read(NAME_LEN_BITS)? as usize;
                if len == 0 || len > MAX_ITEM_NAME_BYTES {
                    return Err(WireError::Malformed);
                }
                for b in names[i].iter_mut().take(len) {
                    *b = r.read(8)? as u8;
                }
                lens[i] = len as u8;
            }
            EventMsg::Catalog {
                total: total as u8,
                first: first as u8,
                count: count as u8,
                names,
                lens,
            }
        }
        SUB_WEAK_MARK => EventMsg::WeakMark {
            cx: r.read(16)? as u16,
            cz: r.read(16)? as u16,
            mark8: r.read(8)? as u8,
            weak_hit: r.read_bit()?,
        },
        SUB_CRAFT_Q => {
            let count = r.read(CRAFT_Q_COUNT_BITS)? as usize;
            if count > CRAFT_QUEUE {
                return Err(WireError::Malformed);
            }
            let mut jobs = [(0u8, 0u8); CRAFT_QUEUE];
            for j in jobs.iter_mut().take(count) {
                let recipe = r.read(8)? as u8;
                let remaining = r.read(8)? as u8;
                if recipe as usize >= MAX_RECIPES || remaining == 0 {
                    return Err(WireError::Malformed);
                }
                *j = (recipe, remaining);
            }
            EventMsg::CraftQ {
                jobs,
                count: count as u8,
                eta_ticks: r.read(16)? as u16,
            }
        }
        SUB_CRAFT_DONE => EventMsg::CraftDone {
            item: r.read(16)? as u16,
            added: r.read(16)? as u16,
        },
        SUB_CRAFT_REFUSED => EventMsg::CraftRefused {
            reason: r.read(8)? as u8,
        },
        SUB_RECIPES => {
            let total = r.read(RECIPE_TOTAL_BITS)? as usize;
            let first = r.read(RECIPE_TOTAL_BITS)? as usize;
            let count = r.read(RECIPE_COUNT_BITS)? as usize;
            if total > MAX_RECIPES || count == 0 || count > RECIPE_BATCH || first + count > total {
                return Err(WireError::Malformed);
            }
            let mut rows = [RecipeDef::INERT; RECIPE_BATCH];
            for row in rows.iter_mut().take(count) {
                let output = r.read(16)? as u16;
                let out_count = r.read(8)? as u16;
                let ticks = r.read(RECIPE_TICKS_BITS)?;
                let station = r.read(STATION_BITS)? as u8;
                let n_inputs = r.read(N_INPUTS_BITS)? as u8;
                if out_count == 0
                    || ticks == 0
                    || station > STATION_FURNACE
                    || n_inputs == 0
                    || n_inputs as usize > MAX_RECIPE_INPUTS
                {
                    return Err(WireError::Malformed);
                }
                let mut inputs = [(0u16, 0u16); MAX_RECIPE_INPUTS];
                for input in inputs.iter_mut().take(n_inputs as usize) {
                    *input = (r.read(16)? as u16, r.read(16)? as u16);
                }
                *row = RecipeDef {
                    output,
                    out_count,
                    ticks,
                    station,
                    n_inputs,
                    inputs,
                };
            }
            EventMsg::Recipes {
                total: total as u8,
                first: first as u8,
                count: count as u8,
                rows,
            }
        }
        SUB_PIECE_PLACED => EventMsg::PiecePlaced {
            rec: read_piece_rec(&mut r)?,
        },
        SUB_PIECE_SYNC => {
            let reset = r.read_bit()?;
            let count = r.read(PIECE_SYNC_COUNT_BITS)? as usize;
            if count > PIECE_SYNC_BATCH || (count == 0 && !reset) {
                return Err(WireError::Malformed);
            }
            let mut recs = [PieceRec::default(); PIECE_SYNC_BATCH];
            for rec in recs.iter_mut().take(count) {
                *rec = read_piece_rec(&mut r)?;
            }
            EventMsg::PieceSync {
                reset,
                recs,
                count: count as u8,
            }
        }
        SUB_BUILD_REFUSED => EventMsg::BuildRefused {
            reason: r.read(8)? as u8,
        },
        SUB_PIECE_DEFS => {
            let total = r.read(PIECE_DEFS_TOTAL_BITS)? as usize;
            let first = r.read(PIECE_DEFS_TOTAL_BITS)? as usize;
            let count = r.read(PIECE_DEFS_COUNT_BITS)? as usize;
            if total > MAX_PIECE_DEFS
                || count == 0
                || count > PIECE_DEFS_BATCH
                || first + count > total
            {
                return Err(WireError::Malformed);
            }
            let mut rows = [PieceDef::INERT; PIECE_DEFS_BATCH];
            for row in rows.iter_mut().take(count) {
                let shape = r.read(SHAPE_BITS)? as u8;
                let material = r.read(MATERIAL_BITS)? as u8;
                let hp = r.read(16)? as u16;
                let n_costs = r.read(N_COSTS_BITS)? as u8;
                if shape > SHAPE_ROOF
                    || material > MAT_METAL
                    || hp == 0
                    || n_costs == 0
                    || n_costs as usize > MAX_PIECE_COSTS
                {
                    return Err(WireError::Malformed);
                }
                let mut costs = [(0u16, 0u16); MAX_PIECE_COSTS];
                for cost in costs.iter_mut().take(n_costs as usize) {
                    *cost = (r.read(16)? as u16, r.read(16)? as u16);
                }
                *row = PieceDef {
                    shape,
                    material,
                    hp,
                    n_costs,
                    costs,
                };
            }
            EventMsg::PieceDefs {
                total: total as u8,
                first: first as u8,
                count: count as u8,
                rows,
            }
        }
        SUB_DEPLOY_PLACED => EventMsg::DeployPlaced {
            rec: read_deploy_rec(&mut r)?,
        },
        SUB_DEPLOY_SYNC => {
            let reset = r.read_bit()?;
            let count = r.read(DEPLOY_SYNC_COUNT_BITS)? as usize;
            if count > DEPLOY_SYNC_BATCH || (count == 0 && !reset) {
                return Err(WireError::Malformed);
            }
            let mut recs = [DeployRec::default(); DEPLOY_SYNC_BATCH];
            for rec in recs.iter_mut().take(count) {
                *rec = read_deploy_rec(&mut r)?;
            }
            EventMsg::DeploySync {
                reset,
                recs,
                count: count as u8,
            }
        }
        SUB_DEPLOY_REFUSED => EventMsg::DeployRefused {
            reason: r.read(8)? as u8,
        },
        SUB_DEPLOY_DEFS => {
            let total = r.read(DEPLOY_DEFS_TOTAL_BITS)? as usize;
            let first = r.read(DEPLOY_DEFS_TOTAL_BITS)? as usize;
            let count = r.read(DEPLOY_DEFS_COUNT_BITS)? as usize;
            if total > MAX_DEPLOY_DEFS
                || count == 0
                || count > DEPLOY_DEFS_BATCH
                || first + count > total
            {
                return Err(WireError::Malformed);
            }
            let mut rows = [DeployDef::INERT; DEPLOY_DEFS_BATCH];
            for row in rows.iter_mut().take(count) {
                let arch = r.read(ARCH_BITS)? as u8;
                let placement = r.read(PLACEMENT_BITS)? as u8;
                let hp = r.read(16)? as u16;
                let item = r.read(16)? as u16;
                if arch > ARCH_DOOR || hp == 0 {
                    return Err(WireError::Malformed);
                }
                *row = DeployDef {
                    arch,
                    placement,
                    hp,
                    item,
                };
            }
            EventMsg::DeployDefs {
                total: total as u8,
                first: first as u8,
                count: count as u8,
                rows,
            }
        }
        sub @ (SUB_PIECE_REMOVED | SUB_DEPLOY_REMOVED) => {
            let cx = r.read(BUILD_CELL_BITS)? as u16;
            let cz = r.read(BUILD_CELL_BITS)? as u16;
            let level = r.read(BUILD_LEVEL_BITS)? as u8;
            let loc = r.read(BUILD_LOC_BITS)? as u8;
            if sub == SUB_PIECE_REMOVED {
                EventMsg::PieceRemoved { cx, cz, level, loc }
            } else {
                EventMsg::DeployRemoved { cx, cz, level, loc }
            }
        }
        SUB_STOCK => {
            let cx = r.read(BUILD_CELL_BITS)? as u16;
            let cz = r.read(BUILD_CELL_BITS)? as u16;
            let level = r.read(BUILD_LEVEL_BITS)? as u8;
            let count = r.read(STOCK_COUNT_BITS)? as usize;
            if count > HEARTH_STOCK_ROWS {
                return Err(WireError::Malformed);
            }
            let mut rows = [(0u16, 0u32); HEARTH_STOCK_ROWS];
            for row in rows.iter_mut().take(count) {
                *row = (r.read(16)? as u16, r.read(32)?);
            }
            EventMsg::Stock {
                cx,
                cz,
                level,
                rows,
                count: count as u8,
            }
        }
        // Address + state, every width exact — nothing to forge.
        SUB_DOOR => EventMsg::Door {
            cx: r.read(BUILD_CELL_BITS)? as u16,
            cz: r.read(BUILD_CELL_BITS)? as u16,
            level: r.read(BUILD_LEVEL_BITS)? as u8,
            loc: r.read(BUILD_LOC_BITS)? as u8,
            open: r.read_bit()?,
            locked: r.read_bit()?,
        },
        _ => return Err(WireError::Malformed),
    };
    expect_zero_padding(&mut r)?;
    Ok(msg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gather_and_slot_change_round_trip() {
        let mut buf = [0u8; MAX_EVENT_MSG_BYTES];
        let len = encode_event_gather(7, 13, &mut buf).unwrap();
        assert_eq!(
            decode_event(&buf[..len]).unwrap(),
            EventMsg::Gather { item: 7, added: 13 }
        );
        let len = encode_event_slot_change(true, 130, 77, &mut buf).unwrap();
        assert_eq!(
            decode_event(&buf[..len]).unwrap(),
            EventMsg::SlotHarvested { cx: 130, cz: 77 }
        );
        let len = encode_event_slot_change(false, 130, 77, &mut buf).unwrap();
        assert_eq!(
            decode_event(&buf[..len]).unwrap(),
            EventMsg::SlotRespawned { cx: 130, cz: 77 }
        );
    }

    #[test]
    fn inv_round_trips_and_refuses_bad_shapes() {
        let mut buf = [0u8; MAX_EVENT_MSG_BYTES];
        let mut slots = [InvSlot::default(); INV_SLOTS];
        for (i, s) in slots.iter_mut().enumerate() {
            *s = InvSlot {
                slot: i as u8,
                stack: ItemStack {
                    item: i as u16,
                    count: 100 + i as u16,
                },
            };
        }
        let len = encode_event_inv(&slots, &mut buf).unwrap();
        match decode_event(&buf[..len]).unwrap() {
            EventMsg::Inv { slots: got, count } => {
                assert_eq!(count as usize, INV_SLOTS);
                assert_eq!(got, slots);
            }
            other => panic!("wrong variant: {other:?}"),
        }
        assert_eq!(encode_event_inv(&[], &mut buf), Err(WireError::Cap));
        let bad = [InvSlot {
            slot: INV_SLOTS as u8,
            stack: ItemStack::default(),
        }];
        assert_eq!(encode_event_inv(&bad, &mut buf), Err(WireError::Range));
    }

    #[test]
    fn slot_sync_full_batch_fits_the_cap() {
        let mut buf = [0u8; MAX_EVENT_MSG_BYTES];
        let cells: [(u16, u16); SLOT_SYNC_BATCH] =
            core::array::from_fn(|i| (i as u16, (i * 3) as u16));
        let len = encode_event_slot_sync(true, &cells, &mut buf).unwrap();
        assert!(len <= MAX_EVENT_MSG_BYTES);
        match decode_event(&buf[..len]).unwrap() {
            EventMsg::SlotSync {
                reset,
                cells: got,
                count,
            } => {
                assert!(reset);
                assert_eq!(count as usize, SLOT_SYNC_BATCH);
                assert_eq!(got, cells);
            }
            other => panic!("wrong variant: {other:?}"),
        }
        // Empty is only a message when it resets.
        assert!(encode_event_slot_sync(true, &[], &mut buf).is_ok());
        assert_eq!(
            encode_event_slot_sync(false, &[], &mut buf),
            Err(WireError::Cap)
        );
    }

    #[test]
    fn catalog_batches_walk_the_table_within_cap() {
        let mut cat = ItemCatalog::EMPTY;
        cat.count = 11;
        for i in 0..11usize {
            // Worst-width names so the cap check is honest.
            let name = [b'a' + (i as u8 % 26); MAX_ITEM_NAME_BYTES];
            cat.set(i, &name).unwrap();
        }
        let mut buf = [0u8; MAX_EVENT_MSG_BYTES];
        let (len, took) = encode_event_catalog(&cat, 0, &mut buf).unwrap();
        assert!(len <= MAX_EVENT_MSG_BYTES);
        assert_eq!(took, CATALOG_BATCH);
        match decode_event(&buf[..len]).unwrap() {
            EventMsg::Catalog {
                total,
                first,
                count,
                names,
                lens,
            } => {
                assert_eq!((total, first, count), (11, 0, CATALOG_BATCH as u8));
                assert_eq!(&names[0][..lens[0] as usize], cat.name(0));
            }
            other => panic!("wrong variant: {other:?}"),
        }
        let (_, took2) = encode_event_catalog(&cat, took, &mut buf).unwrap();
        assert_eq!(took + took2, 11);
        assert_eq!(
            encode_event_catalog(&cat, 11, &mut buf),
            Err(WireError::Range)
        );
    }

    #[test]
    fn weak_mark_round_trips_both_flag_states() {
        let mut buf = [0u8; MAX_EVENT_MSG_BYTES];
        for weak_hit in [false, true] {
            let len = encode_event_weak_mark(0x0141, 0x0087, 0xC3, weak_hit, &mut buf).unwrap();
            assert_eq!(
                decode_event(&buf[..len]).unwrap(),
                EventMsg::WeakMark {
                    cx: 0x0141,
                    cz: 0x0087,
                    mark8: 0xC3,
                    weak_hit,
                }
            );
        }
    }

    #[test]
    fn craft_q_round_trips_and_refuses_bad_jobs() {
        let mut buf = [0u8; MAX_EVENT_MSG_BYTES];
        let jobs = [
            CraftJob {
                recipe: 3,
                remaining: 99,
            },
            CraftJob {
                recipe: 0,
                remaining: 1,
            },
        ];
        let len = encode_event_craft_q(&jobs, 1234, &mut buf).unwrap();
        match decode_event(&buf[..len]).unwrap() {
            EventMsg::CraftQ {
                jobs: got,
                count,
                eta_ticks,
            } => {
                assert_eq!(count, 2);
                assert_eq!(&got[..2], &[(3, 99), (0, 1)]);
                assert_eq!(eta_ticks, 1234);
            }
            other => panic!("wrong variant: {other:?}"),
        }
        // Empty is the "queue cleared" message.
        let len = encode_event_craft_q(&[], 0, &mut buf).unwrap();
        match decode_event(&buf[..len]).unwrap() {
            EventMsg::CraftQ { count, .. } => assert_eq!(count, 0),
            other => panic!("wrong variant: {other:?}"),
        }
        // A dead job, an over-u8 remaining, an out-of-table recipe: Range.
        for bad in [
            CraftJob {
                recipe: 0,
                remaining: 0,
            },
            CraftJob {
                recipe: 0,
                remaining: 256,
            },
            CraftJob {
                recipe: MAX_RECIPES as u16,
                remaining: 1,
            },
        ] {
            assert_eq!(
                encode_event_craft_q(&[bad], 0, &mut buf),
                Err(WireError::Range)
            );
        }
    }

    #[test]
    fn craft_done_and_refused_round_trip() {
        let mut buf = [0u8; MAX_EVENT_MSG_BYTES];
        let len = encode_event_craft_done(9, 3, &mut buf).unwrap();
        assert_eq!(
            decode_event(&buf[..len]).unwrap(),
            EventMsg::CraftDone { item: 9, added: 3 }
        );
        let len = encode_event_craft_refused(4, &mut buf).unwrap();
        assert_eq!(
            decode_event(&buf[..len]).unwrap(),
            EventMsg::CraftRefused { reason: 4 }
        );
    }

    #[test]
    fn recipes_batches_walk_the_table_within_cap() {
        let cc = CraftContent::probe_fixture();
        let mut buf = [0u8; MAX_EVENT_MSG_BYTES];
        let (len, took) = encode_event_recipes(&cc, 0, &mut buf).unwrap();
        assert!(len <= MAX_EVENT_MSG_BYTES);
        assert_eq!(took, 3, "fixture has 3 rows, all fit one batch");
        match decode_event(&buf[..len]).unwrap() {
            EventMsg::Recipes {
                total,
                first,
                count,
                rows,
            } => {
                assert_eq!((total, first, count), (3, 0, 3));
                assert_eq!(rows[0], cc.recipes[0], "decode rebuilds the sim row");
                assert_eq!(rows[1], cc.recipes[1]);
                assert_eq!(rows[2], cc.recipes[2]);
            }
            other => panic!("wrong variant: {other:?}"),
        }
        assert_eq!(
            encode_event_recipes(&cc, 3, &mut buf),
            Err(WireError::Range),
            "cursor past the table refuses"
        );
        // A row the bake would refuse (zero ticks) refuses here.
        let mut bad = cc;
        bad.recipes[1].ticks = 0;
        assert_eq!(
            encode_event_recipes(&bad, 0, &mut buf),
            Err(WireError::Range)
        );
    }

    #[test]
    fn recipes_full_batch_fits_the_cap() {
        // Worst shape: RECIPE_BATCH rows, every input row live at u16 max.
        let mut cc = CraftContent::EMPTY;
        cc.recipe_count = RECIPE_BATCH as u16 + 1;
        for i in 0..cc.recipe_count as usize {
            cc.recipes[i] = RecipeDef {
                output: u16::MAX,
                out_count: 255,
                ticks: 65_535 * sim_core::limits::TICK_HZ,
                station: STATION_FURNACE,
                n_inputs: MAX_RECIPE_INPUTS as u8,
                inputs: [(u16::MAX, u16::MAX); MAX_RECIPE_INPUTS],
            };
        }
        let mut buf = [0u8; MAX_EVENT_MSG_BYTES];
        let (len, took) = encode_event_recipes(&cc, 0, &mut buf).unwrap();
        assert!(len <= MAX_EVENT_MSG_BYTES);
        assert_eq!(took, RECIPE_BATCH);
        let (_, took2) = encode_event_recipes(&cc, took, &mut buf).unwrap();
        assert_eq!(took + took2, cc.recipe_count as usize);
    }

    #[test]
    fn piece_placed_and_build_refused_round_trip() {
        let mut buf = [0u8; MAX_EVENT_MSG_BYTES];
        let rec = PieceRec {
            cx: 341,
            cz: 682,
            level: 3,
            loc: LOC_EDGE_N,
            row: 17,
            ..PieceRec::default()
        };
        let len = encode_event_piece_placed(&rec, &mut buf).unwrap();
        assert_eq!(
            decode_event(&buf[..len]).unwrap(),
            EventMsg::PiecePlaced { rec }
        );
        // A record the sim could never hold refuses at encode.
        let bad = PieceRec {
            row: MAX_PIECE_DEFS as u8,
            ..rec
        };
        assert_eq!(
            encode_event_piece_placed(&bad, &mut buf),
            Err(WireError::Range)
        );
        let len = encode_event_build_refused(4, &mut buf).unwrap();
        assert_eq!(
            decode_event(&buf[..len]).unwrap(),
            EventMsg::BuildRefused { reason: 4 }
        );
    }

    #[test]
    fn piece_sync_full_batch_fits_the_cap() {
        let mut buf = [0u8; MAX_EVENT_MSG_BYTES];
        let recs: [PieceRec; PIECE_SYNC_BATCH] = core::array::from_fn(|i| PieceRec {
            cx: i as u16 * 31,
            cz: 1023 - i as u16,
            level: (i % MAX_BUILD_LEVELS) as u8,
            loc: (i % 4) as u8,
            row: (i % MAX_PIECE_DEFS) as u8,
            ..PieceRec::default()
        });
        let len = encode_event_piece_sync(true, &recs, &mut buf).unwrap();
        assert!(len <= MAX_EVENT_MSG_BYTES);
        match decode_event(&buf[..len]).unwrap() {
            EventMsg::PieceSync {
                reset,
                recs: got,
                count,
            } => {
                assert!(reset);
                assert_eq!(count as usize, PIECE_SYNC_BATCH);
                assert_eq!(got, recs);
            }
            other => panic!("wrong variant: {other:?}"),
        }
        // Empty is only a message when it resets.
        assert!(encode_event_piece_sync(true, &[], &mut buf).is_ok());
        assert_eq!(
            encode_event_piece_sync(false, &[], &mut buf),
            Err(WireError::Cap)
        );
    }

    #[test]
    fn piece_defs_batches_walk_the_table_within_cap() {
        let bc = BuildContent::probe_fixture();
        let mut buf = [0u8; MAX_EVENT_MSG_BYTES];
        let (len, took) = encode_event_piece_defs(&bc, 0, &mut buf).unwrap();
        assert!(len <= MAX_EVENT_MSG_BYTES);
        assert_eq!(took, 4, "fixture has 4 rows, all fit one batch");
        match decode_event(&buf[..len]).unwrap() {
            EventMsg::PieceDefs {
                total,
                first,
                count,
                rows,
            } => {
                assert_eq!((total, first, count), (4, 0, 4));
                assert_eq!(rows[0], bc.pieces[0], "decode rebuilds the sim row");
                assert_eq!(rows[3], bc.pieces[3]);
            }
            other => panic!("wrong variant: {other:?}"),
        }
        assert_eq!(
            encode_event_piece_defs(&bc, 4, &mut buf),
            Err(WireError::Range),
            "cursor past the table refuses"
        );
        // A row the bake would refuse (hp 0) refuses here.
        let mut bad = bc;
        bad.pieces[1].hp = 0;
        assert_eq!(
            encode_event_piece_defs(&bad, 0, &mut buf),
            Err(WireError::Range)
        );
        // The full 18-row alpha shape drips in three batches.
        let mut full = BuildContent::EMPTY;
        full.piece_count = 18;
        for i in 0..18 {
            full.pieces[i] = PieceDef {
                shape: (i % 6) as u8,
                material: (i % 3) as u8,
                hp: u16::MAX,
                n_costs: MAX_PIECE_COSTS as u8,
                costs: [(u16::MAX, u16::MAX); MAX_PIECE_COSTS],
            };
        }
        let (len, took) = encode_event_piece_defs(&full, 0, &mut buf).unwrap();
        assert!(len <= MAX_EVENT_MSG_BYTES);
        assert_eq!(took, PIECE_DEFS_BATCH);
        let (_, took2) = encode_event_piece_defs(&full, took, &mut buf).unwrap();
        let (_, took3) = encode_event_piece_defs(&full, took + took2, &mut buf).unwrap();
        assert_eq!(took + took2 + took3, 18);
    }

    #[test]
    fn deploy_placed_sync_and_refused_round_trip() {
        let mut buf = [0u8; MAX_EVENT_MSG_BYTES];
        let rec = DeployRec {
            cx: 341,
            cz: 682,
            level: 1,
            loc: sim_core::build::LOC_PLANE,
            row: 7,
            ..DeployRec::default()
        };
        let len = encode_event_deploy_placed(&rec, &mut buf).unwrap();
        assert_eq!(
            decode_event(&buf[..len]).unwrap(),
            EventMsg::DeployPlaced { rec },
            "owner/hp/uh stay sim-side: decode carries defaults"
        );
        // A record the sim could never hold refuses at encode.
        let bad = DeployRec {
            row: MAX_DEPLOY_DEFS as u8,
            ..rec
        };
        assert_eq!(
            encode_event_deploy_placed(&bad, &mut buf),
            Err(WireError::Range)
        );

        // A full sync batch fits the cap; empty only resets.
        let recs: [DeployRec; DEPLOY_SYNC_BATCH] = core::array::from_fn(|i| DeployRec {
            cx: i as u16 * 41,
            cz: 1023 - i as u16,
            level: (i % MAX_BUILD_LEVELS) as u8,
            loc: (i % 4) as u8,
            row: (i % MAX_DEPLOY_DEFS) as u8,
            ..DeployRec::default()
        });
        let len = encode_event_deploy_sync(true, &recs, &mut buf).unwrap();
        assert!(len <= MAX_EVENT_MSG_BYTES);
        match decode_event(&buf[..len]).unwrap() {
            EventMsg::DeploySync {
                reset,
                recs: got,
                count,
            } => {
                assert!(reset);
                assert_eq!(count as usize, DEPLOY_SYNC_BATCH);
                assert_eq!(got, recs);
            }
            other => panic!("wrong variant: {other:?}"),
        }
        assert!(encode_event_deploy_sync(true, &[], &mut buf).is_ok());
        assert_eq!(
            encode_event_deploy_sync(false, &[], &mut buf),
            Err(WireError::Cap)
        );

        let len = encode_event_deploy_refused(7, &mut buf).unwrap();
        assert_eq!(
            decode_event(&buf[..len]).unwrap(),
            EventMsg::DeployRefused { reason: 7 }
        );
    }

    #[test]
    fn deploy_defs_batches_walk_the_table_within_cap() {
        let dc = DeployContent::probe_fixture();
        let mut buf = [0u8; MAX_EVENT_MSG_BYTES];
        let (len, took) = encode_event_deploy_defs(&dc, 0, &mut buf).unwrap();
        assert!(len <= MAX_EVENT_MSG_BYTES);
        assert_eq!(took, 4, "fixture has 4 rows, all fit one batch");
        match decode_event(&buf[..len]).unwrap() {
            EventMsg::DeployDefs {
                total,
                first,
                count,
                rows,
            } => {
                assert_eq!((total, first, count), (4, 0, 4));
                assert_eq!(rows[0], dc.defs[0], "decode rebuilds the sim row");
                assert_eq!(rows[3], dc.defs[3]);
            }
            other => panic!("wrong variant: {other:?}"),
        }
        assert_eq!(
            encode_event_deploy_defs(&dc, 4, &mut buf),
            Err(WireError::Range),
            "cursor past the table refuses"
        );
        // A row the bake would refuse (hp 0) refuses here.
        let mut bad = dc;
        bad.defs[1].hp = 0;
        assert_eq!(
            encode_event_deploy_defs(&bad, 0, &mut buf),
            Err(WireError::Range)
        );
        // The 16-row cap shape drips in two batches.
        let mut full = DeployContent::EMPTY;
        full.def_count = MAX_DEPLOY_DEFS as u16;
        for i in 0..MAX_DEPLOY_DEFS {
            full.defs[i] = DeployDef {
                arch: (i % 7) as u8,
                placement: (i % 4) as u8,
                hp: u16::MAX,
                item: u16::MAX,
            };
        }
        let (len, took) = encode_event_deploy_defs(&full, 0, &mut buf).unwrap();
        assert!(len <= MAX_EVENT_MSG_BYTES);
        assert_eq!(took, DEPLOY_DEFS_BATCH);
        let (_, took2) = encode_event_deploy_defs(&full, took, &mut buf).unwrap();
        assert_eq!(took + took2, MAX_DEPLOY_DEFS);
    }

    #[test]
    fn removals_and_stock_round_trip() {
        let mut buf = [0u8; MAX_EVENT_MSG_BYTES];
        for piece in [true, false] {
            let len = encode_event_removed(piece, 341, 682, 2, LOC_EDGE_N, &mut buf).unwrap();
            let want = if piece {
                EventMsg::PieceRemoved {
                    cx: 341,
                    cz: 682,
                    level: 2,
                    loc: LOC_EDGE_N,
                }
            } else {
                EventMsg::DeployRemoved {
                    cx: 341,
                    cz: 682,
                    level: 2,
                    loc: LOC_EDGE_N,
                }
            };
            assert_eq!(decode_event(&buf[..len]).unwrap(), want);
        }
        assert_eq!(
            encode_event_removed(true, 1024, 0, 0, 0, &mut buf),
            Err(WireError::Range)
        );

        // Stock at the row cap round-trips; past it refuses.
        let rows = [(0u16, 2_000u32), (5, 0), (9, 123_456), (63, u32::MAX)];
        let len = encode_event_stock(341, 682, 0, &rows, &mut buf).unwrap();
        match decode_event(&buf[..len]).unwrap() {
            EventMsg::Stock {
                cx,
                cz,
                level,
                rows: got,
                count,
            } => {
                assert_eq!((cx, cz, level), (341, 682, 0));
                assert_eq!(count as usize, HEARTH_STOCK_ROWS);
                assert_eq!(got, rows);
            }
            other => panic!("wrong variant: {other:?}"),
        }
        let too_many = [(0u16, 0u32); HEARTH_STOCK_ROWS + 1];
        assert_eq!(
            encode_event_stock(0, 0, 0, &too_many, &mut buf),
            Err(WireError::Cap)
        );
    }

    #[test]
    fn trailing_garbage_and_unknown_subtype_are_malformed() {
        let mut buf = [0u8; MAX_EVENT_MSG_BYTES];
        let len = encode_event_gather(1, 2, &mut buf).unwrap();
        assert_eq!(
            decode_event(&buf[..len + 1]),
            Err(WireError::Malformed),
            "spare byte after a valid message must fail the strict tail"
        );
        // kind EVENT + subtype 23: unknown (the first unused subtype).
        let raw = [
            (KIND_EVENT | (23 << KIND_BITS)) as u8,
            (23 >> (8 - KIND_BITS)) as u8,
        ];
        assert_eq!(decode_event(&raw[..2]), Err(WireError::Malformed));
    }
}
