//! Loose items lying on the ground — what a smashed barrel leaves behind.
//!
//! **The operator's call** (2026-09-16: *"when u break a barrel it drops
//! loot as 3d objects… we could just make it generic now but eventually
//! loot can fall down and out of reach kinda thing. or roll a bit"*), and
//! it is the reference's own split: a thing you **break** scatters, a
//! thing that **dies** leaves a container (`reference/LOOT.md` §1, §9.3).
//! So this module is the barrel's new home and `backpack.rs` keeps the
//! death drop, the spill and the corpse.
//!
//! ⚠ **It reverses a spoken row rather than filling a gap**, which is why
//! the reversal is written down here as well as in `DECISIONS.md`. The
//! 2026-08 row *"barrel loot lands in the ground-container store"* chose a
//! bag on purpose — *"a second store would have bought a second wire
//! message, a second sync walk and a second eviction policy for one
//! difference nothing downstream reads"* — and ended with the condition
//! that undoes it: *"raise the cap or split the stores if that is wrong."*
//! It is wrong for a reason that row could not weigh, because it is about
//! the frame rather than the store: **a bag is one object wherever the
//! loot came from**, so a barrel and a corpse read identically at ten
//! metres, and the thing you break tells you nothing about what fell out
//! of it. The three costs it named are all real and all paid: this file is
//! the second store, `SUB_GITEM_SYNC` the second sync walk, and
//! [`GroundItems::scatter`] carries the second eviction policy.
//!
//! ## What is here and what is deliberately not
//!
//! A record is **one stack at rest**: an address, what it is, and when it
//! goes. There is no velocity, no per-tick integration and nothing in
//! flight — the *"roll"* the operator defers is a **landing spot** here,
//! drawn deterministically around the container that paid it
//! ([`rest_spot`]), so loot already lands downhill, on a shelf, or over an
//! edge. What a later slice adds is the tumble a player can *watch*, and
//! `reference/LOOT.md` §9.3 states the one rule that slice must keep: the
//! settle stays the sim's, because a client-side tumble over a
//! server-side resting place is two truths about a thing you can pick up.
//!
//! Also absent, each because something already owns it: no despawn ladder
//! of its own (`BackpackContent::stack_life_ticks`, so a rare stack lies
//! there as long as a rare bag would), no reach of its own
//! (`backpack::LOOT_REACH_M`, the arm every world interaction shares), and
//! no take event of its own (`EV_GATHER` — "this entered your hands", the
//! currency a bag's loot and a node's yield are already paid in).
//!
//! Pure and fixed-capacity like the rest of the crate: one pass over at
//! most `MAX_GROUND_ITEMS` live entries per sweep, no allocation after
//! construction, and every value integer or `f32` on the restricted
//! operator set.

use crate::backpack::LOOT_REACH_M;
use crate::gather::{inv_add_spilling_skinned, GatherContent, ItemStack};
use crate::limits::{INV_SLOTS, MAX_GROUND_ITEMS};
use crate::movement::{quant_xz, quant_y, POS_XZ_Q};
use crate::rng::{cell_hash, splitmix64};
use crate::terrain::{self, Haven};
use crate::world::{EventQueue, Player, EV_GATHER};

/// Noise channel for the scatter offsets. Its own, above the worldgen
/// band (< 96) and clear of `gather`'s 97/98, `loot`'s 99 and
/// `worldcont`'s 100 — two containers bursting in one cell must not draw
/// the same offsets, and a shared channel is how they would.
pub const CH_SCATTER: u32 = 101;

/// How far from the container a stack may come to rest, metres — **the
/// one number this module adds**, and it is a square rather than a disc
/// (the offsets are two independent draws, so the corner reaches
/// `SCATTER_R_M * √2`). A disc would want a reject loop or a second
/// `sqrt` for no difference a player can see.
///
/// Sized against the two things it has to sit between: large enough that
/// two stacks out of one barrel are visibly separate objects rather than
/// one pile, and small enough that every stack a container paid is inside
/// one standing spot's `LOOT_REACH_M` (5 m) — 1.7 m worst case against
/// 5 m, so a player never has to walk to collect what one swing broke.
/// `DECISIONS.md` §open, ground items v0.
pub const SCATTER_R_M: f32 = 1.2;

/// One stack at rest. Position is quantized in the same quanta a body's
/// is (`movement.rs`) — the sim sims on the values it transmits, so the
/// wire carries these ints and the client never draws an item somewhere
/// the sim will not let it be taken.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct GroundItemRec {
    /// Shard-unique, monotonic from 1. Zero is "no item".
    pub id: u32,
    pub qx: i32,
    pub qy: i32,
    pub qz: i32,
    pub stack: ItemStack,
    /// Tick at which it despawns.
    pub expires: u64,
}

/// Every loose stack on the island.
///
/// Dense, swap-removed, and **boxed** — `MAX_GROUND_ITEMS` records of
/// 28 bytes is under the line that turned `test_parity_wasm` into an
/// out-of-bounds read, but the next widening of `ItemStack` is exactly
/// how `Backpacks` crossed it (`CLAUDE.md`'s shadow-stack trap), and a
/// store that boxes from the first commit never has to be converted.
pub struct GroundItems {
    entries: Box<[GroundItemRec; MAX_GROUND_ITEMS]>,
    len: usize,
    /// Next item id. Sim state, hashed: two replays of one WAL must name
    /// the same stack the same thing.
    next_id: u32,
}

impl Default for GroundItems {
    fn default() -> Self {
        Self::new()
    }
}

impl GroundItems {
    pub fn new() -> Self {
        Self {
            entries: crate::boxed_array(GroundItemRec::default()),
            len: 0,
            next_id: 1,
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The live records, dense from 0.
    pub fn entries(&self) -> &[GroundItemRec] {
        &self.entries[..self.len]
    }

    /// Where the item with this id sits, or `None` if it is gone. Ids are
    /// never reused (`next_id` is monotonic and saturates), so this cannot
    /// answer with a different stack that took the same index.
    pub fn index_of_id(&self, id: u32) -> Option<usize> {
        if id == 0 {
            return None;
        }
        self.entries[..self.len].iter().position(|e| e.id == id)
    }

    /// Scatter a container's roll onto the ground around `(qx, qz)`.
    ///
    /// One record per non-empty stack, each at its own [`rest_spot`], and
    /// `items` is **left untouched** — unlike `Backpacks::spill_at`, whose
    /// contract is to clear what it took. The difference is deliberate and
    /// it is the one the merge-gate judge caught on that function: a
    /// buffer that is both an input and an output has to be cleared by
    /// every path or it duplicates into the world, and the way not to have
    /// that problem is not to take an output buffer. The barrel's roll is
    /// a fresh array the caller owns for the tick, so this reads it.
    ///
    /// **Overflow evicts the record nearest its own despawn**, which is
    /// `MAX_BACKPACKS`' policy and for `MAX_BACKPACKS`' reason: refusing
    /// would make a full island quietly eat a barrel somebody paid three
    /// swings for, and the entry about to vanish anyway is the cheapest
    /// one to lose. Unlike `MAX_WORLD_CONTS`, evicting here cannot dupe —
    /// a record holds no timer anybody can re-roll.
    ///
    /// Returns how many stacks reached the ground.
    #[allow(clippy::too_many_arguments)]
    pub fn scatter(
        &mut self,
        bc: &crate::backpack::BackpackContent,
        seed: u64,
        haven: &Haven,
        key: u32,
        qx: i32,
        qz: i32,
        items: &[ItemStack; INV_SLOTS],
        tick: u64,
    ) -> usize {
        if bc.base_ticks == 0 {
            // Inert content disarms this module exactly as it disarms the
            // bag: a shard whose ladder was never authored gets the
            // pre-loot world rather than litter with no lifetime.
            return 0;
        }
        let mut made = 0usize;
        for (k, stack) in items.iter().enumerate() {
            if stack.count == 0 {
                continue;
            }
            if self.len == MAX_GROUND_ITEMS {
                let mut worst = 0usize;
                for i in 1..self.len {
                    if self.entries[i].expires < self.entries[worst].expires {
                        worst = i;
                    }
                }
                self.remove(worst);
            }
            let (ix, iy, iz) = rest_spot(seed, haven, key, k, qx, qz);
            let id = self.next_id;
            // Saturates rather than wraps: a wrapped id would alias an
            // early one and a take would resolve to the wrong stack.
            self.next_id = self.next_id.saturating_add(1);
            let i = self.len;
            self.entries[i] = GroundItemRec {
                id,
                qx: ix,
                qy: iy,
                qz: iz,
                stack: *stack,
                expires: tick + bc.stack_life_ticks(stack.item) as u64,
            };
            self.len += 1;
            made += 1;
        }
        made
    }

    /// Drop record `i`, swap-removing so the store stays dense.
    fn remove(&mut self, i: usize) {
        if i >= self.len {
            return;
        }
        self.len -= 1;
        self.entries[i] = self.entries[self.len];
        self.entries[self.len] = GroundItemRec::default();
    }

    /// Retire every stack whose timer ran out. One pass over the live
    /// entries, taken every tick; the swap-remove is why the index does
    /// not advance on a hit.
    ///
    /// **Silent, unlike a bag's despawn.** `EV_BAG_REMOVED` exists because
    /// a bag is a container a player may have open and may be walking
    /// back to; a loose stack is scenery, the client's next sync simply
    /// does not carry it, and an event per expiring stack would spend the
    /// lane on litter.
    pub fn expire_due(&mut self, tick: u64) -> usize {
        let mut gone = 0usize;
        let mut i = 0;
        while i < self.len {
            if self.entries[i].expires <= tick {
                self.remove(i);
                gone += 1;
            } else {
                i += 1;
            }
        }
        gone
    }

    /// Is record `i` within arm's reach of `p`? Planar, like every other
    /// world interaction's reach test.
    pub fn in_reach(&self, i: usize, p: &Player) -> bool {
        if i >= self.len {
            return false;
        }
        let dx = self.entries[i].qx as f32 * POS_XZ_Q - p.body.qx as f32 * POS_XZ_Q;
        let dz = self.entries[i].qz as f32 * POS_XZ_Q - p.body.qz as f32 * POS_XZ_Q;
        dx * dx + dz * dz <= LOOT_REACH_M * LOOT_REACH_M
    }

    /// The nearest loose stack within reach of `p`, or `None`.
    ///
    /// Nearest-in-reach, ties to the lower index — `loot_nearest`'s own
    /// rule, and the reason the take action carries **no payload**: the
    /// sim picks, so nothing aimable at a stack the player is not standing
    /// on ever crosses the wire.
    pub fn nearest(&self, p: &Player) -> Option<usize> {
        let mut best: Option<(usize, f32)> = None;
        for i in 0..self.len {
            if !self.in_reach(i, p) {
                continue;
            }
            let dx = self.entries[i].qx as f32 * POS_XZ_Q - p.body.qx as f32 * POS_XZ_Q;
            let dz = self.entries[i].qz as f32 * POS_XZ_Q - p.body.qz as f32 * POS_XZ_Q;
            let d2 = dx * dx + dz * dz;
            match best {
                Some((_, bd2)) if bd2 <= d2 => {}
                _ => best = Some((i, d2)),
            }
        }
        best.map(|(i, _)| i)
    }

    /// Take the nearest loose stack in reach into `p`'s inventory.
    ///
    /// All-or-part: what fits goes in the pack, what does not goes into
    /// `spill` for `World::drain_spill` to catch — the same buffer
    /// contract `gather::swing` and `craft::step` keep, so this verb owns
    /// no container store either. A stack only partly taken **stays on
    /// the ground with the remainder**, which is the bag's rule
    /// (`loot_nearest`) and the honest one: the alternative is a take that
    /// silently destroys what it could not carry.
    ///
    /// The take announces `EV_GATHER`, so it pays in the `+N Item` toast
    /// a node's yield and a bag's loot already pay in.
    pub fn take_nearest(
        &mut self,
        gc: &GatherContent,
        p: &mut Player,
        spill: &mut [ItemStack; INV_SLOTS],
        events: &mut EventQueue,
    ) -> Option<u32> {
        let i = self.nearest(p)?;
        let rec = self.entries[i];
        let cap = gc.stack_max_of(rec.stack.item);
        // An existing stack: its condition and its skin travel with it.
        let took = inv_add_spilling_skinned(
            &mut p.inv,
            spill,
            rec.stack.item,
            rec.stack.count,
            cap,
            rec.stack.cond,
            rec.stack.skin,
        );
        // `inv_add_spilling_skinned` puts the remainder in `spill`, so the whole
        // stack has left the ground whether the pack held it or not —
        // which is why the record goes rather than being decremented. A
        // ceiling of zero (`REFUSE_M_UNSTACKABLE`'s condition) takes
        // nothing anywhere, and that is the one case the record must
        // survive: an item no ladder can size would otherwise vanish off
        // the island by being looked at.
        if took == 0 && spill.iter().all(|s| s.count == 0) {
            return None;
        }
        events.push(
            EV_GATHER,
            p.id,
            ((rec.stack.item as u32) << 16) | took as u32,
            0,
        );
        self.remove(i);
        Some(rec.id)
    }

    /// Install a record verbatim — the save loader's door, and nothing
    /// else's. `next_id` is advanced past every id it installs so a
    /// reboot cannot mint a second stack with a live id.
    pub fn restore(&mut self, rec: GroundItemRec) -> bool {
        if self.len == MAX_GROUND_ITEMS || rec.id == 0 || rec.stack.count == 0 {
            return false;
        }
        self.entries[self.len] = rec;
        self.len += 1;
        if rec.id >= self.next_id {
            self.next_id = rec.id.saturating_add(1);
        }
        true
    }

    /// The next id this store will mint. Save/replay material — it is
    /// hashed state, so a reader has to be able to see it.
    pub fn next_id(&self) -> u32 {
        self.next_id
    }
}

/// Where the `k`-th stack out of the container keyed `key` at `(qx, qz)`
/// comes to rest, quantized.
///
/// **Pure, and the whole of the operator's *"fall down and out of reach"*
/// in v0.** The offset is two integer draws off one hash, so the same
/// barrel on the same tick scatters identically on every replay and on
/// both targets (wall 1 and wall 5); the height is `terrain::ground`, the
/// same surface a body walks on, so a stack that lands over a lip lies
/// where the lip is — downhill, on a shelf, or at the foot of a drop the
/// player has to walk around. Nothing in the sim *moves* it there; the
/// landing spot simply is where the arithmetic says.
///
/// `k` is mixed in rather than the tick: two stacks out of one container
/// must land apart, and they are rolled on the same tick.
pub fn rest_spot(
    seed: u64,
    haven: &Haven,
    key: u32,
    k: usize,
    qx: i32,
    qz: i32,
) -> (i32, i32, i32) {
    let h = splitmix64(cell_hash(seed, key as i32, k as i32, CH_SCATTER));
    // Quanta, from metres, at the position quantum the wire carries.
    // `max(1)` so a radius small enough to quantize to zero still spreads
    // by one quantum rather than stacking every item on one point.
    let span = quant_xz(SCATTER_R_M).max(1);
    let width = 2 * span + 1;
    let dx = (h & 0xFFFF) as i32 % width - span;
    let dz = ((h >> 16) & 0xFFFF) as i32 % width - span;
    let ix = qx + dx;
    let iz = qz + dz;
    let x = ix as f32 * POS_XZ_Q;
    let z = iz as f32 * POS_XZ_Q;
    (ix, quant_y(terrain::ground(seed, haven, x, z)), iz)
}
