//! World containers: the crate on the haven pad and the cache at a
//! waystation — the two authored things a player walks to.
//!
//! **Why this is a store and the barrel is not.** A smashed barrel stands
//! up a `backpack.rs` record (`DECISIONS.md`, "barrel loot lands in the
//! ground-container store") because a barrel *stops existing* when it pays:
//! it has an address for as long as its loot lies on the ground and then it
//! is gone. A crate is the opposite — it is furniture that outlives being
//! emptied, it stands at a cell terrain authored rather than at a position
//! something died at, and it must still be there, and still openable, on
//! the next visit. A bag record cannot say that: it carries an `owner` and
//! an `expires`, and the whole of its lifetime is a countdown.
//!
//! So the record here is the smallest thing that can: **the cell it stands
//! at, the table it rolls, what is currently inside, and when it may roll
//! again.**
//!
//! **Three things this deliberately does not do.**
//!
//! 1. **It does not place anything.** `terrain::scatter` already places the
//!    pad's five crates and the waystations' caches, as a pure function of
//!    `(seed, cell)`. A second list of where the containers are would be a
//!    second answer to a question terrain already answers, and the two
//!    would drift. A record is minted the first time somebody opens one,
//!    and `open` re-derives the cell through `scatter` to prove the ask.
//! 2. **It does not tick.** There is no sweep and no per-tick work of any
//!    kind. An emptied container carries the tick it may roll again, and
//!    the refill happens inside `open` — the one moment anybody can tell
//!    the difference. A sweep would visit 64 records every tick to change
//!    something no player is looking at.
//! 3. **It does not invent a move verb.** `CONT_WORLD` is a third ground
//!    container kind, so `inventory::plan_move` resolves a crate exactly
//!    the way it resolves a box, with the same refusals and the same
//!    per-tick `ContSync` diff. Not one line of `plan_move` moved for this.

use crate::gather::{cell_key, GatherContent, ItemStack, RESPAWN_MIN_TICKS, RESPAWN_RANGE_TICKS};
use crate::limits::{INV_SLOTS, MAX_WORLD_CONTS};
use crate::loot::{LootContent, LOOT_CACHE, LOOT_CRATE};
use crate::movement::POS_XZ_Q;
use crate::rng::{cell_hash, splitmix64};
use crate::terrain::{self, Occupant};
use crate::world::Player;

/// Reach for opening and for every move against a world container — the
/// same arm a bag, a box, a door and a hearth feed already share
/// (`build::BUILD_REACH_M` by way of `backpack::LOOT_REACH_M`). Reused,
/// never re-spoken: a crate is not a special distance.
pub use crate::backpack::LOOT_REACH_M;

/// Noise channel for the refill jitter. Its own, not `CH_RESPAWN`'s: a
/// waystation cache and a barrel can stand in the same cell, and sharing
/// the channel would make the two come back on the same jitter draw —
/// correlated by an accident of the key rather than by anything the
/// design says.
pub const CH_REFILL: u32 = 100;

/// How long the container at `(cx, cz)`, emptied on `tick`, waits before
/// it may roll again.
///
/// **The barrel's own window, reused rather than re-spoken** —
/// `gather::RESPAWN_{MIN,RANGE}_TICKS`, which `DECISIONS.md` §open records
/// as "node/barrel respawn 20–45 min" and which already names two things.
/// A crate is the third, and it wants what they want: long enough that
/// clearing the pad is worth doing, jittered so a shard does not learn one
/// clock. Nothing here is a new number; if the window is wrong it is wrong
/// for all three, which is the right place for that argument.
pub fn refill_ticks(seed: u64, cx: u16, cz: u16, tick: u64) -> u64 {
    let jitter = splitmix64(cell_hash(seed, cx as i32, cz as i32, CH_REFILL) ^ tick);
    RESPAWN_MIN_TICKS + jitter % RESPAWN_RANGE_TICKS
}

/// One authored container standing at a scatter cell.
///
/// `qx`/`qz` are the quantized stand position, copied off the `Slot` the
/// first open derived. They are stored rather than re-derived because the
/// server re-proves reach **every tick** a panel is open (`drip_client`),
/// and `terrain::scatter` is ~60 `noise2` evaluations — `occupy.rs`'s own
/// measurement, and the reason `gather::swing` reads through a memo. A
/// per-tick cold scatter per open panel is exactly the spike that file
/// exists to refuse.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct WorldContRec {
    pub cx: u16,
    pub cz: u16,
    /// Quantized stand position (`POS_XZ_Q`), for the reach check.
    pub qx: i32,
    pub qz: i32,
    /// Which `LOOT_*` table this cell rolls. Derived from the occupant, so
    /// a crate cannot be made to pay a cache's table by anything on the
    /// wire.
    pub table: u8,
    /// Tick at which an emptied container may roll again. `0` means it is
    /// not empty, or has never been emptied.
    pub refill_at: u64,
    pub items: [ItemStack; INV_SLOTS],
}

impl WorldContRec {
    pub fn is_empty(&self) -> bool {
        self.items.iter().all(|s| s.count == 0)
    }
}

/// Which loot table an occupant rolls, or `None` for a cell that is not a
/// world container at all. The whole of the authorization: a cell the
/// client names that does not answer here cannot be opened.
pub fn table_of(o: Occupant) -> Option<usize> {
    match o {
        Occupant::CrateSlot => Some(LOOT_CRATE),
        Occupant::CacheSlot => Some(LOOT_CACHE),
        _ => None,
    }
}

/// Dense, insertion-ordered, fixed capacity — `Backpacks`' shape.
///
/// Unlike every other container store there is **no removal path**, so no
/// swap-remove and no sync-walk restart contract: an authored container
/// does not stop existing. Records only ever appear (first open) and empty
/// out.
pub struct WorldConts {
    entries: Box<[WorldContRec; MAX_WORLD_CONTS]>,
    len: usize,
}

impl WorldConts {
    pub fn new() -> Self {
        Self {
            // Heap-filled, never a stack frame: `MAX_WORLD_CONTS` records
            // of `INV_SLOTS` stacks is ~8.7 KB, and wasm32's shadow stack
            // is 1 MiB with no guard page (`crate::boxed_array`, and the
            // three times this trap has already been paid).
            entries: crate::boxed_array(WorldContRec::default()),
            len: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn entries(&self) -> &[WorldContRec] {
        &self.entries[..self.len]
    }

    /// Replace the store from a decoded world save. Boot-only
    /// (`worldsave.rs`).
    pub(crate) fn restore(&mut self, recs: &[WorldContRec]) {
        self.len = recs.len().min(MAX_WORLD_CONTS);
        self.entries[..self.len].copy_from_slice(&recs[..self.len]);
    }

    /// Index of the container at `handle`, where `handle` is
    /// `gather::cell_key(cx, cz)`.
    ///
    /// Handle `0` never resolves. It is cell `(0, 0)`'s key and it is also
    /// the "nothing open" value the container subscription uses, so it is
    /// guarded here exactly as `deploy::box_index` guards `box_key`'s zero
    /// — one cell in the far corner of a 256×256 grid is unaddressable, and
    /// saying so is cheaper than a second handle space.
    pub fn index_of(&self, handle: u32) -> Option<usize> {
        if handle == 0 {
            return None;
        }
        self.entries[..self.len]
            .iter()
            .position(|c| cell_key(c.cx, c.cz) == handle)
    }

    /// Is index `i` within arm's reach of `p`? Asked at the moment a move
    /// resolves and again on every tick a panel is open — never trusted
    /// from the open, the same contract a bag carries.
    pub fn in_reach(&self, i: usize, p: &Player) -> bool {
        if i >= self.len {
            return false;
        }
        let dx = self.entries[i].qx as f32 * POS_XZ_Q - p.body.qx as f32 * POS_XZ_Q;
        let dz = self.entries[i].qz as f32 * POS_XZ_Q - p.body.qz as f32 * POS_XZ_Q;
        dx * dx + dz * dz <= LOOT_REACH_M * LOOT_REACH_M
    }

    /// Read one slot. Out of range reads as empty — total, so a forged
    /// index is a refusal upstream and never an index panic here.
    pub fn slot(&self, i: usize, s: usize) -> ItemStack {
        if i >= self.len || s >= INV_SLOTS {
            return ItemStack::default();
        }
        self.entries[i].items[s]
    }

    /// Write one slot. Out of range writes nothing.
    ///
    /// Emptying the last stack arms the refill timer here rather than in
    /// the move verb, because this is the one function every write goes
    /// through: a container emptied by a take and a container emptied by
    /// any future verb both land on the same line.
    pub fn set_slot(&mut self, i: usize, s: usize, v: ItemStack, tick: u64, refill_ticks: u64) {
        if i >= self.len || s >= INV_SLOTS {
            return;
        }
        self.entries[i].items[s] = v;
        if self.entries[i].is_empty() {
            // Only arm it on the transition. Re-arming on every write to an
            // already-empty container would let a player hold the refill
            // off forever by putting one item in and taking it back out.
            if self.entries[i].refill_at == 0 {
                self.entries[i].refill_at = tick + refill_ticks;
            }
        } else {
            self.entries[i].refill_at = 0;
        }
    }

    /// Open the container at cell `(cx, cz)` on behalf of `p`, minting the
    /// record and rolling its contents if this is the first visit — or if
    /// it was emptied and its refill tick has come.
    ///
    /// Returns the resolved index, or `None` with nothing written. The
    /// three refusals are all silent by design: the container simply does
    /// not open, and the client's panel closes on the next tick's
    /// `ContSync(CONT_SELF)` — the same thing that happens when a bag
    /// despawns out from under an open panel, so it needs no new sentence
    /// and no new event.
    ///
    /// 1. **The cell is not a container.** The occupant is re-derived
    ///    through `terrain::scatter`, so the client's handle is checked
    ///    against the world rather than believed.
    /// 2. **Out of reach**, by the same arm every other container uses.
    /// 3. **The store is full.** Refuse rather than evict: an empty record
    ///    carries a refill timer, so evicting one and re-minting it would
    ///    re-roll the loot immediately, which turns the cap into a dupe.
    ///    `MAX_WORLD_CONTS` is 7× what the island authors, and the
    ///    unreachability of this branch is the point of `the_cap_is_above
    ///    _what_terrain_authors`.
    #[allow(clippy::too_many_arguments)]
    pub fn open(
        &mut self,
        seed: u64,
        scatter: &terrain::ScatterTable,
        haven: &terrain::Haven,
        lc: &LootContent,
        gc: &GatherContent,
        tick: u64,
        cx: u16,
        cz: u16,
        p: &Player,
    ) -> Option<usize> {
        let handle = cell_key(cx, cz);
        if handle == 0 {
            return None;
        }
        let slot = terrain::scatter(seed, scatter, haven, cx as i32, cz as i32);
        let table = table_of(slot.occupant)?;

        let qx = (slot.x * (1.0 / POS_XZ_Q)) as i32;
        let qz = (slot.z * (1.0 / POS_XZ_Q)) as i32;
        let dx = qx as f32 * POS_XZ_Q - p.body.qx as f32 * POS_XZ_Q;
        let dz = qz as f32 * POS_XZ_Q - p.body.qz as f32 * POS_XZ_Q;
        if dx * dx + dz * dz > LOOT_REACH_M * LOOT_REACH_M {
            return None;
        }

        let i = match self.index_of(handle) {
            Some(i) => i,
            None => {
                if self.len >= MAX_WORLD_CONTS {
                    return None;
                }
                let i = self.len;
                self.entries[i] = WorldContRec {
                    cx,
                    cz,
                    qx,
                    qz,
                    table: table as u8,
                    // Rolled below, on the same path a refill takes, so
                    // "first open" and "opened again after it refilled"
                    // are one code path and cannot diverge.
                    refill_at: 0,
                    items: [ItemStack::default(); INV_SLOTS],
                };
                self.len += 1;
                self.roll(i, lc, gc, seed, handle, tick);
                return Some(i);
            }
        };

        // An emptied container whose tick has come rolls again, here, on
        // the visit that notices. `refill_at == 0` is "not empty", which
        // is why an untouched container never re-rolls.
        if self.entries[i].refill_at != 0 && tick >= self.entries[i].refill_at {
            self.entries[i].refill_at = 0;
            self.roll(i, lc, gc, seed, handle, tick);
        }
        Some(i)
    }

    /// Fill record `i` from its table. Same `roll_into` the barrel uses, so
    /// a crate and a barrel draw through one weighted walk and one bias
    /// correction — the tables differ, the arithmetic does not.
    fn roll(
        &mut self,
        i: usize,
        lc: &LootContent,
        gc: &GatherContent,
        seed: u64,
        handle: u32,
        tick: u64,
    ) {
        let table = self.entries[i].table as usize;
        self.entries[i].items = [ItemStack::default(); INV_SLOTS];
        // `tick` is mixed into the draw exactly as it is for a barrel, so a
        // container that refills pays a different roll rather than the same
        // one forever — and a replayed WAL re-rolls it identically, because
        // a replay opens it on the same tick.
        let mut out = [ItemStack::default(); INV_SLOTS];
        lc.roll_into(table, gc, seed, handle, tick, &mut out);
        self.entries[i].items = out;
    }
}

impl Default for WorldConts {
    fn default() -> Self {
        Self::new()
    }
}
