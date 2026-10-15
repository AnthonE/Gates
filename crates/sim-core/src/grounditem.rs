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
//! edge. `reference/LOOT.md` §9.3 states the one rule a tumble a player can
//! *watch* must keep: the settle stays the sim's, because a client-side
//! tumble over a server-side resting place is two truths about a thing you
//! can pick up.
//!
//! **A dropped stack is thrown** (operator, 2026-10-15: *"when people pull
//! stuff out from there inventory in rust it just spits the 3d model out to
//! roll around and settle"*), and the throw keeps that rule by being a
//! **closed form** rather than an integration — Before Colony's way with
//! its colony's traffic: the sim picks where the stack lands
//! ([`throw_spot`]: out of the hand along the look, stopped by anything an
//! arrow would stop on, down onto the first surface under it) and records
//! the throw beside the record ([`Toss`]), and the flight between the hand
//! and the landing is one function of the tick ([`toss_at`]) that the sim
//! and every client evaluate alike. Nothing is integrated per tick and
//! nothing in flight is stored but its two ends; the landing is the record.
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
use crate::limits::{INV_SLOTS, MAX_GROUND_ITEMS, MAX_TOSSES, TICK_HZ};
use crate::movement::{quant_xz, quant_y, POS_XZ_Q, POS_Y_Q};
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
    /// An arrow **stuck** where it went in (`spent::SpentRec::dir`): the
    /// way it was flying, a byte an axis, and the position is where it
    /// went in rather than the ground under it. Zero for everything that
    /// lies on the ground.
    pub dir: [i8; 3],
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
    /// The stacks still in the air ([`Toss`]).
    tosses: Tosses,
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
            tosses: Tosses::default(),
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
            let (ix, iy, iz) = rest_spot(seed, haven, key, k, qx, qz);
            self.push(bc, (ix, iy, iz), *stack, tick, [0; 3]);
            made += 1;
        }
        made
    }

    /// Lay one stack down at a spot already found — an arrow falling out
    /// of the air or out of a body (`spent::settle`, [`rest_at`]), or one
    /// stuck where it went in, flying `dir` ([`stick_at`]; zero for one
    /// that lies). Disarmed by inert content exactly as
    /// [`scatter`](Self::scatter) is. Returns the new record's id, or 0 if
    /// nothing was laid.
    pub fn drop_one(
        &mut self,
        bc: &crate::backpack::BackpackContent,
        at: (i32, i32, i32),
        stack: ItemStack,
        tick: u64,
        dir: [i8; 3],
    ) -> u32 {
        if bc.base_ticks == 0 || stack.count == 0 {
            return 0;
        }
        self.push(bc, at, stack, tick, dir)
    }

    /// Throw one stack out of a hand at `from` (quanta) to rest at `at`
    /// ([`throw_spot`]): [`drop_one`](Self::drop_one), and the throw
    /// remembered for as long as it flies. A throw too long for the wire's
    /// offsets ([`TOSS_XZ_MAX_Q`], [`TOSS_Y_MAX_Q`]) — off a cliff — is not
    /// remembered, and the stack is simply there. Returns the record's id,
    /// or 0 if nothing was laid.
    pub fn toss(
        &mut self,
        bc: &crate::backpack::BackpackContent,
        at: (i32, i32, i32),
        stack: ItemStack,
        tick: u64,
        from: (i32, i32, i32),
    ) -> u32 {
        let id = self.drop_one(bc, at, stack, tick, [0; 3]);
        let off = (from.0 - at.0, from.1 - at.1, from.2 - at.2);
        let fits = |v: i32, max: i32| (-max..=max).contains(&v);
        if id != 0
            && fits(off.0, TOSS_XZ_MAX_Q)
            && fits(off.1, TOSS_Y_MAX_Q)
            && fits(off.2, TOSS_XZ_MAX_Q)
        {
            self.tosses.push(Toss {
                id,
                born: tick,
                from: [off.0 as i16, off.1 as i16, off.2 as i16],
            });
        }
        id
    }

    /// The throw record `id` is still flying on at `tick`, or `None` once it
    /// has landed (or was never thrown). What the loose-stack sync puts on
    /// the wire beside the record, so a client draws the same flight.
    pub fn toss_of(&self, id: u32, tick: u64) -> Option<Toss> {
        self.tosses.of(id, tick)
    }

    /// A stuck arrow whose trunk, wall or door has gone falls to `at`
    /// ([`rest_at`]), or is lost when that is `None`. Its despawn starts
    /// again from `expires`, as Rust's does when an arrow falls out.
    ///
    /// **It comes down as a new stack, with a new id.** The loose-stack
    /// sync is keyed on `(next_id, len)` and a client draws a stack where
    /// it first saw it, so a record that moved under its old id would go
    /// on standing in the air on every screen; a fresh id is a change the
    /// walk already restarts on, and the client's set redraws it where it
    /// fell.
    pub fn fall(&mut self, i: usize, at: Option<(i32, i32, i32)>, expires: u64) {
        if i >= self.len {
            return;
        }
        let Some((qx, qy, qz)) = at else {
            self.remove(i);
            return;
        };
        let e = &mut self.entries[i];
        (e.qx, e.qy, e.qz) = (qx, qy, qz);
        e.dir = [0; 3];
        e.expires = expires;
        e.id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
    }

    /// Append one record, evicting the one nearest its own despawn when
    /// the store is full (`scatter`'s stated policy). Returns its id.
    fn push(
        &mut self,
        bc: &crate::backpack::BackpackContent,
        (qx, qy, qz): (i32, i32, i32),
        stack: ItemStack,
        tick: u64,
        dir: [i8; 3],
    ) -> u32 {
        if self.len == MAX_GROUND_ITEMS {
            let mut worst = 0usize;
            for i in 1..self.len {
                if self.entries[i].expires < self.entries[worst].expires {
                    worst = i;
                }
            }
            self.remove(worst);
        }
        let id = self.next_id;
        // Saturates rather than wraps: a wrapped id would alias an early
        // one and a take would resolve to the wrong stack.
        self.next_id = self.next_id.saturating_add(1);
        self.entries[self.len] = GroundItemRec {
            id,
            qx,
            qy,
            qz,
            stack,
            expires: tick + bc.stack_life_ticks(stack.item) as u64,
            dir,
        };
        self.len += 1;
        id
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
        self.nearest_where(p, |_| true)
    }

    /// [`Self::nearest`] over the stacks `clear` passes as well — the world's
    /// hand test (`reach.rs`), asked of each quantized position after the
    /// planar reach.
    pub fn nearest_where(
        &self,
        p: &Player,
        clear: impl Fn(&GroundItemRec) -> bool,
    ) -> Option<usize> {
        // Nearest first, the hand's sight asked of as few as find one
        // (`reach::nearest_clear`); ties to the lower index, as ever.
        let mut cand = [(0.0f32, 0u16); MAX_GROUND_ITEMS];
        let mut n = 0;
        for i in 0..self.len {
            if !self.in_reach(i, p) {
                continue;
            }
            let dx = self.entries[i].qx as f32 * POS_XZ_Q - p.body.qx as f32 * POS_XZ_Q;
            let dz = self.entries[i].qz as f32 * POS_XZ_Q - p.body.qz as f32 * POS_XZ_Q;
            cand[n] = (dx * dx + dz * dz, i as u16);
            n += 1;
        }
        crate::reach::nearest_clear(&mut cand[..n], |i| clear(&self.entries[i]))
    }

    /// The stack a take from `p` lifts: `target` (a stack id) when it is
    /// still lying in reach, else [`Self::nearest`]. `target` is what the
    /// player's crosshair named, so the prompt and the take agree; it is
    /// only a preference, never a reach past the arm.
    pub fn pick(&self, p: &Player, target: u32) -> Option<usize> {
        self.index_of_id(target)
            .filter(|&i| self.in_reach(i, p))
            .or_else(|| self.nearest(p))
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
        self.take_nearest_where(gc, p, spill, events, |_| true)
    }

    /// [`Self::take_nearest`] over [`Self::nearest_where`].
    pub fn take_nearest_where(
        &mut self,
        gc: &GatherContent,
        p: &mut Player,
        spill: &mut [ItemStack; INV_SLOTS],
        events: &mut EventQueue,
        clear: impl Fn(&GroundItemRec) -> bool,
    ) -> Option<u32> {
        let i = self.nearest_where(p, clear)?;
        self.take_at(i, gc, p, spill, events)
    }

    /// [`Self::take_nearest`] for the stack at index `i`, which the caller
    /// has already found in reach ([`Self::pick`]).
    pub fn take_at(
        &mut self,
        i: usize,
        gc: &GatherContent,
        p: &mut Player,
        spill: &mut [ItemStack; INV_SLOTS],
        events: &mut EventQueue,
    ) -> Option<u32> {
        if i >= self.len {
            return None;
        }
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
        // One event for both halves: `took` reached the pack, c went to the
        // feet (a part fit raises the `+N` toast and the spill line both).
        events.push(
            EV_GATHER,
            p.id,
            ((rec.stack.item as u32) << 16) | took as u32,
            (rec.stack.count - took) as u32,
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

/// A stack thrown out of a hand (`Command::Drop`): which record, the tick
/// it left the hand, and where the hand was **relative to where it comes to
/// rest**, in body quanta (`POS_XZ_Q` across, `POS_Y_Q` up). The rest is
/// the record; this is only its flight, and [`toss_at`] is the flight.
///
/// **Neither saved nor hashed, and that is a claim about what it is.** It
/// describes the next second of a picture and no tick reads it: a reload
/// lands every stack, which is what a stack does a second later anyway,
/// and `tests/worldsave.rs` refuses a reload that moves `state_hash` —
/// so a field the save does not carry may not be folded into the hash.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Toss {
    pub id: u32,
    pub born: u64,
    pub from: [i16; 3],
}

impl Toss {
    /// Ticks from the hand to the ground.
    pub fn ticks(&self) -> u32 {
        toss_ticks(self.from[1] as f32 * POS_Y_Q)
    }
}

/// A point in body quanta: `POS_XZ_Q` across, `POS_Y_Q` up.
pub type Quanta = (i32, i32, i32);

/// The widest throw the wire carries, quanta: 15 m across, 20 m down.
pub const TOSS_XZ_MAX_Q: i32 = 511;
pub const TOSS_Y_MAX_Q: i32 = 2047;

/// The throws still in the air, oldest overwritten first (`MAX_TOSSES`).
#[derive(Clone, Copy, Debug)]
pub struct Tosses {
    recs: [Toss; MAX_TOSSES],
    next: usize,
}

impl Default for Tosses {
    fn default() -> Self {
        Self {
            recs: [Toss::default(); MAX_TOSSES],
            next: 0,
        }
    }
}

impl Tosses {
    fn push(&mut self, t: Toss) {
        self.recs[self.next] = t;
        self.next = (self.next + 1) % MAX_TOSSES;
    }

    fn of(&self, id: u32, tick: u64) -> Option<Toss> {
        if id == 0 {
            return None;
        }
        self.recs
            .iter()
            .find(|t| t.id == id)
            .filter(|t| tick < t.born + t.ticks() as u64)
            .copied()
    }
}

/// Gravity a thrown stack falls under, m/s².
pub const TOSS_G: f32 = 9.8;
/// How fast a throw leaves the hand upward, m/s: a toss, not a lob.
pub const TOSS_UP_MPS: f32 = 1.6;
/// How far in front of the body a level look throws, and how near a look
/// at the feet does, metres, along the ground.
pub const TOSS_REACH_M: f32 = 1.6;
pub const TOSS_NEAR_M: f32 = 0.5;
/// Where the hand lets go: below the eye, and in front of the chest.
pub const TOSS_HAND_DROP_M: f32 = 0.4;
pub const TOSS_HAND_OUT_M: f32 = 0.3;
/// How far short of whatever stops a throw it lands, metres: a stack does
/// not rest inside the wall it hit.
pub const TOSS_BACKOFF_M: f32 = 0.25;
/// The throw's own extent against the world, metres — a stack is fatter
/// than an arrowhead.
pub const TOSS_PROBE_M: f32 = 0.08;

/// How many ticks a throw is in the air when its hand is `drop_m` above
/// where it lands: up at [`TOSS_UP_MPS`], down under [`TOSS_G`]. A landing
/// above the hand (never, off [`throw_spot`]) is clamped to the apex.
pub fn toss_ticks(drop_m: f32) -> u32 {
    let up = TOSS_UP_MPS;
    let disc = (up * up + 2.0 * TOSS_G * drop_m).max(0.0);
    let secs = (up + disc.sqrt()) / TOSS_G;
    (secs * TICK_HZ as f32) as u32 + 1
}

/// Where a throw from `from` to `to` (metres) is `t` ticks after it left
/// the hand: a straight line in plan, and in height the parabola under
/// [`TOSS_G`] through both ends in `ticks` ([`toss_ticks`]), so it lands
/// exactly on the record. Clamped at both ends.
pub fn toss_at(from: (f32, f32, f32), to: (f32, f32, f32), ticks: u32, t: f32) -> (f32, f32, f32) {
    let ticks = ticks.max(1) as f32;
    let s = (t / ticks).clamp(0.0, 1.0);
    let secs = ticks / TICK_HZ as f32;
    let ts = s * secs;
    // The upward speed that lands it at `to.1` after `secs`.
    let vy = (to.1 - from.1 + 0.5 * TOSS_G * secs * secs) / secs;
    (
        from.0 + (to.0 - from.0) * s,
        from.1 + vy * ts - 0.5 * TOSS_G * ts * ts,
        from.2 + (to.2 - from.2) * s,
    )
}

/// Where a stack thrown out of the hand of a body at `(qx, qy, qz)` with
/// its eye `eye_m` above its feet, looking along yaw `yaw` and pitch
/// `pitch`, leaves the hand and comes to rest, both in quanta:
/// `(from, rest)`.
///
/// The throw goes out along the look in plan, [`TOSS_REACH_M`] on a level
/// look and [`TOSS_NEAR_M`] on one at the feet, at the hand's height, and
/// stops [`TOSS_BACKOFF_M`] short of the first thing an arrow would stop
/// on — `ranged::world_stop`'s own ladder, so a stack goes through no wall,
/// crate or rock a shot could not. It lands on the first surface under
/// that point ([`rest_at`]). `None` when there is none — the sea, or off
/// the island — and the caller drops it at the feet as it always did.
#[allow(clippy::too_many_arguments)]
pub fn throw_spot(
    seed: u64,
    haven: &Haven,
    cols: &crate::collide::ColIndex,
    occ: &mut crate::occupy::Occupants,
    (qx, qy, qz): (i32, i32, i32),
    eye_m: f32,
    yaw: u16,
    pitch: u8,
) -> Option<(Quanta, Quanta)> {
    use crate::ranged::MM_PER_M;
    let (fx, fz) = crate::yaw_dir(yaw);
    let (ch, sv) = crate::pitch_dir(pitch);
    // A level look throws furthest, a look down at the feet nearest, and a
    // look up no further than level.
    let reach = TOSS_NEAR_M + (TOSS_REACH_M - TOSS_NEAR_M) * (ch + sv).clamp(0.0, 1.0);
    let (bx, by, bz) = (
        qx as f32 * POS_XZ_Q,
        qy as f32 * POS_Y_Q,
        qz as f32 * POS_XZ_Q,
    );
    let hand_y = by + eye_m - TOSS_HAND_DROP_M;
    // Walked from the body's own axis rather than from the hand, so a body
    // with its face to a wall does not let go on the far side of it.
    let n = (reach / 0.1) as usize + 1;
    let (t, surf, _) = crate::ranged::world_stop(
        seed,
        haven,
        cols,
        occ,
        (bx * MM_PER_M, hand_y * MM_PER_M, bz * MM_PER_M),
        (fx * reach * MM_PER_M, 0.0, fz * reach * MM_PER_M),
        n,
        n,
        TOSS_PROBE_M,
    );
    let free = if surf.is_some() {
        (t * reach - TOSS_BACKOFF_M).max(0.0)
    } else {
        reach
    };
    let out = TOSS_HAND_OUT_M.min(free);
    let from = (
        quant_xz(bx + fx * out),
        quant_y(hand_y),
        quant_xz(bz + fz * out),
    );
    let rest = rest_at(
        seed,
        haven,
        cols,
        occ,
        bx + fx * free,
        hand_y,
        bz + fz * free,
    )?;
    Some((from, rest))
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

/// Deeper than this under the sea, a thing that falls is lost rather than
/// left on the seabed.
pub const SINK_DEPTH_M: f32 = 0.5;

/// Where an arrow stuck at `(x, y, z)` metres is kept, in body quanta: the
/// point itself, not the ground under it. `None` off the island or in
/// water deeper than [`SINK_DEPTH_M`], as [`rest_at`].
pub fn stick_at(x: f32, y: f32, z: f32) -> Option<(i32, i32, i32)> {
    if !(0.0..terrain::ISLAND_SIZE).contains(&x) || !(0.0..terrain::ISLAND_SIZE).contains(&z) {
        return None;
    }
    if y < terrain::SEA_LEVEL - SINK_DEPTH_M {
        return None;
    }
    Some((quant_xz(x), quant_y(y), quant_xz(z)))
}

/// Where something let go of at `(x, y, z)` metres comes to rest, in body
/// quanta: straight down onto the first surface a body could stand on at
/// that height — terrain, a built floor or stair, a crate, rock or plinth
/// top — which is `movement::step`'s own ground. `None` off the island or
/// in water deeper than [`SINK_DEPTH_M`].
pub fn rest_at(
    seed: u64,
    haven: &Haven,
    cols: &crate::collide::ColIndex,
    occ: &mut crate::occupy::Occupants,
    x: f32,
    y: f32,
    z: f32,
) -> Option<(i32, i32, i32)> {
    if !(0.0..terrain::ISLAND_SIZE).contains(&x) || !(0.0..terrain::ISLAND_SIZE).contains(&z) {
        return None;
    }
    let g = terrain::ground(seed, haven, x, z)
        .max(crate::collide::piece_ground(seed, haven, cols, x, z, y))
        .max(occ.ground(seed, x, z, y));
    if g < terrain::SEA_LEVEL - SINK_DEPTH_M {
        return None;
    }
    Some((quant_xz(x), quant_y(g), quant_xz(z)))
}
