//! The death backpack — the container that makes a kill pay (DESIGN.md
//! §2: "death drops your whole inventory where you fell; bags despawn on
//! a timer"; NETCODE.md §6.4 is the shape).
//!
//! Melee v0 gave the world a way to take a life and nothing to take off
//! the body: `world::respawn` destroyed the inventory outright, because
//! the world had no ground container to catch it. This module is that
//! container, and it is deliberately **one entity, not thirty** — exactly
//! Rust's shipped consolidation (ragdoll, then collapse to a single
//! backpack; Facepunch's stated reason was cost), minus the ragdoll,
//! which is a cosmetic mesh and content for later.
//!
//! Despawn is content's, not code's (CLAUDE.md wall 7): one base constant
//! × the rarity multiplier of the rarest item inside, baked per item into
//! `BackpackContent::despawn_ticks` from `content/balance.toml`'s
//! `[backpack]` table and `content/items.toml`'s rarity column — the
//! column that file has always said "drives the despawn multiplier". An
//! empty ladder (`base_ticks == 0`) is the inert default and disarms the
//! module entirely: death destroys, exactly as it did before this slice.
//!
//! Pure and fixed-capacity like the rest of the crate. Every sweep is one
//! pass over at most `MAX_BACKPACKS` live entries, the store never
//! allocates, and both mutating paths (drop, loot) are integer-and-`f32`
//! arithmetic on the restricted operator set.
//!
//! **What v0 deliberately does not do**, registered in `DECISIONS.md`
//! §open ("death backpack v0"): no per-slot looting (the take is
//! all-that-fits, because a container UI is its own slice and a
//! half-container that can only be emptied is honest where a broken grid
//! is not), no id-targeted loot (the pick is the nearest bag in reach,
//! the same shape `gather::swing` and `combat::strike` use, so nothing
//! spoofable crosses the wire), no bag hp and so no destroying one
//! without opening it, and no player-initiated drop verb (there is no
//! `Command::Drop` — putting a chosen stack on the ground is a different
//! feature from catching one that had nowhere to go).
//!
//! **Ground drops from a full inventory landed 2026-08-14** and were the
//! last line of that list: `spill_at` catches what `inv_add` used to
//! destroy. It went in on the two paths that *pay* a player — a node's
//! yield and a finished craft — and the four that *give one back* took the
//! same lane later the same day: a demolish refund (`build.rs`), a
//! deployable pick-up and a lock removal (`deploy.rs`, `lock.rs`) and a
//! craft cancel's refund (`craft.rs`). **Six producers, one drain**
//! (`World::drain_spill`), and nothing else may call `spill_at` — the
//! owner is named in code because that is what `CLAUDE.md`'s clean-merge
//! trap costs when it is named in a comment instead.
//!
//! So no path in the sim destroys an item because a pack was full. The
//! two things still open are both about *telling* the player: a spill is
//! silent (`EV_GATHER` honestly reports the zero that reached the hands)
//! and the merge ignores ownership. `NOW.md` §0sp2 carries both.

use crate::gather::{inv_add, GatherContent, ItemStack};
use crate::limits::{INV_SLOTS, MAX_BACKPACKS, MAX_ITEM_DEFS};
use crate::movement::POS_XZ_Q;
use crate::world::{EventQueue, Player, EV_BAG_DROPPED, EV_BAG_REMOVED, EV_GATHER};

/// Reach for opening a bag: the same arm every other world interaction
/// uses (`build::BUILD_REACH_M`, which a hearth feed, a door and a lock
/// already share). Not a new knob — deliberately the same one.
pub use crate::build::BUILD_REACH_M as LOOT_REACH_M;

/// Why a bag left the world — the `b` field of `EV_BAG_REMOVED`.
pub const BAG_GONE_DESPAWN: u32 = 0;
pub const BAG_GONE_EMPTIED: u32 = 1;
/// The store was full when someone died: the bag nearest its own despawn
/// made room. NETCODE.md §6.4's "overflow despawns oldest-lowest-tier
/// first" — one key does both, because a bag's expiry already encodes its
/// age *and* its best tier.
pub const BAG_GONE_EVICTED: u32 = 2;
/// The highest reason above — `DEATH_BY_MAX`'s posture for this ledger.
/// The wire field is two bits, so a forged `why == 3` fits the width; the
/// server refuses it against this constant at the encode boundary
/// (`server/core.rs`, NOW.md §5b). A **literal**, unlike `DEATH_BY_MAX`,
/// because `protocol`'s domain scrape reads this block as text and its
/// exempt list is a wire-pass edit; `tests/domain_ledger.rs` fails if the
/// literal stops naming the ledger's top.
pub const BAG_GONE_MAX: u32 = 2;

/// The despawn ladder the sim knows, baked from content.
#[derive(Clone, Copy, Debug)]
pub struct BackpackContent {
    /// Per item index: how long a bag holding this item lives, in ticks.
    /// A bag's own lifetime is the max over what it holds.
    pub despawn_ticks: [u32; MAX_ITEM_DEFS],
    /// Lifetime floor, in ticks — `despawn_base_min` at the tick rate.
    /// **Zero disarms the whole module**: no bag is ever created, and
    /// death destroys the inventory as it did before this slice.
    pub base_ticks: u32,
}

impl BackpackContent {
    pub const EMPTY: Self = Self {
        despawn_ticks: [0; MAX_ITEM_DEFS],
        base_ticks: 0,
    };

    /// Synthetic ladder for the parity/replay/alloc gates, on the same
    /// pattern as the gather and combat fixtures. Short lifetimes on
    /// purpose: 90 ticks (3 s) base so a probe run of a few hundred ticks
    /// actually walks a bag through spawn → loot → despawn instead of
    /// leaving every bag it makes standing at the end.
    pub fn probe_fixture() -> Self {
        let mut c = Self::EMPTY;
        c.base_ticks = 90;
        let mut i = 0;
        while i < MAX_ITEM_DEFS {
            // Items 0..3 are the fixture's "rare" half: four times the
            // floor, so the max-over-contents rule has something to pick.
            c.despawn_ticks[i] = if i < 4 { 360 } else { 90 };
            i += 1;
        }
        c
    }

    /// How long a bag holding `items` lives, in ticks: the base, raised
    /// to the longest-lived thing inside. An empty bag never gets made,
    /// so the base is a floor, not a common case.
    pub fn lifetime_ticks(&self, items: &[ItemStack; INV_SLOTS]) -> u32 {
        let mut life = self.base_ticks;
        for s in items.iter() {
            if s.count == 0 || s.item as usize >= MAX_ITEM_DEFS {
                continue;
            }
            let t = self.despawn_ticks[s.item as usize];
            if t > life {
                life = t;
            }
        }
        life
    }
}

impl Default for BackpackContent {
    fn default() -> Self {
        Self::EMPTY
    }
}

/// One bag on the ground. Position is the dead body's quantized position
/// verbatim (`movement.rs` quanta) — the sim sims on the values it
/// transmits, so the wire carries these ints and the client never rounds
/// a bag somewhere the sim will not open it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BackpackRec {
    /// Shard-unique, monotonic from 1. Zero is "no bag".
    pub id: u32,
    pub qx: i32,
    pub qy: i32,
    pub qz: i32,
    /// Who died. Sim-side only — the wire carries address and id, never
    /// whose it was, the same posture a deployable's owner takes.
    pub owner: u32,
    /// Tick at which the bag despawns.
    pub expires: u64,
    pub items: [ItemStack; INV_SLOTS],
}

impl Default for BackpackRec {
    fn default() -> Self {
        Self {
            id: 0,
            qx: 0,
            qy: 0,
            qz: 0,
            owner: 0,
            expires: 0,
            items: [ItemStack::default(); INV_SLOTS],
        }
    }
}

impl BackpackRec {
    /// True once nothing is left inside — the moment the bag is done.
    pub fn is_empty(&self) -> bool {
        self.items.iter().all(|s| s.count == 0)
    }
}

/// The ground-container store: dense, insertion-ordered, fixed capacity.
/// Removal swap-removes (like `Deploys`), so the wire layer restarts an
/// in-progress sync walk on any removal — the same contract the piece and
/// deploy walks already carry.
pub struct Backpacks {
    entries: [BackpackRec; MAX_BACKPACKS],
    len: usize,
    /// Next bag id. Sim state, hashed: two replays of the same WAL must
    /// name the same bag the same thing.
    next_id: u32,
}

impl Backpacks {
    pub fn new() -> Self {
        Self {
            entries: [BackpackRec::default(); MAX_BACKPACKS],
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

    pub fn entries(&self) -> &[BackpackRec] {
        &self.entries[..self.len]
    }

    pub fn next_id(&self) -> u32 {
        self.next_id
    }

    /// Replace the store from a decoded world save. Boot-only
    /// (`worldsave.rs`).
    ///
    /// `next_id` is restored rather than recomputed from the records, and
    /// the difference matters: recomputing it as `max(id) + 1` would reuse
    /// the ids of bags that have already despawned, so a player who looted
    /// bag 7 before the restart could be handed a different bag 7 after it.
    /// The counter is monotonic state (it is hashed for exactly this
    /// reason), and the decoder refuses any record whose id is at or past
    /// it.
    pub(crate) fn restore(&mut self, recs: &[BackpackRec], next_id: u32) {
        self.len = recs.len().min(MAX_BACKPACKS);
        self.entries[..self.len].copy_from_slice(&recs[..self.len]);
        self.next_id = next_id;
    }

    pub fn find(&self, id: u32) -> Option<&BackpackRec> {
        self.entries[..self.len].iter().find(|b| b.id == id)
    }

    /// Swap-remove index `i`, announcing why.
    fn remove(&mut self, i: usize, why: u32, events: &mut EventQueue) {
        let id = self.entries[i].id;
        self.len -= 1;
        self.entries[i] = self.entries[self.len];
        self.entries[self.len] = BackpackRec::default();
        events.push(EV_BAG_REMOVED, id, why, 0);
    }

    /// Drop `items` where a body fell. Returns the new bag's id, or
    /// `None` when nothing was dropped — an inert ladder (content that
    /// never armed the module) and an empty inventory both take this
    /// exit, so a naked spawn dying leaves no litter.
    ///
    /// Overflow policy: **evict** the bag nearest its own despawn, which
    /// is NETCODE.md §6.4's "oldest-lowest-tier first" collapsed into the
    /// one key that already encodes both. Ties break on the lowest index,
    /// so the choice is a pure function of state.
    pub fn drop_for(
        &mut self,
        bc: &BackpackContent,
        body: &Player,
        tick: u64,
        events: &mut EventQueue,
    ) -> Option<u32> {
        if bc.base_ticks == 0 {
            return None; // inert content: the module is disarmed
        }
        self.stand_up(
            bc,
            body.body.qx,
            body.body.qy + BAG_Y_OFFSET_Q,
            body.body.qz,
            body.id,
            &body.inv,
            tick,
            events,
        )
    }

    /// Stand a container up at a quantized address holding `items`.
    ///
    /// The one insert path: a death bag, a smashed barrel's loot and a
    /// killed animal's corpse (`mob::strike`) are the same object — a
    /// container on the ground with a lifetime, an address, and slots the
    /// move verb already resolves as `CONT_BAG`.
    /// Splitting them into two stores would have bought a second wire
    /// message, a second sync walk and a second eviction policy for one
    /// difference (where the items came from) that nothing downstream
    /// reads: `owner` is state-hash and test material only, never a gate.
    ///
    /// `None` when nothing stood up — an inert ladder (content that never
    /// armed the module) and an empty item set both take this exit, so a
    /// naked spawn dying leaves no litter and neither does a barrel whose
    /// roll paid nothing.
    #[allow(clippy::too_many_arguments)]
    pub fn stand_up(
        &mut self,
        bc: &BackpackContent,
        qx: i32,
        qy: i32,
        qz: i32,
        owner: u32,
        items: &[ItemStack; INV_SLOTS],
        tick: u64,
        events: &mut EventQueue,
    ) -> Option<u32> {
        if bc.base_ticks == 0 {
            return None; // inert content: the module is disarmed
        }
        if items.iter().all(|s| s.count == 0) {
            return None; // nothing to catch
        }
        if self.len == MAX_BACKPACKS {
            let mut worst = 0usize;
            for i in 1..self.len {
                if self.entries[i].expires < self.entries[worst].expires {
                    worst = i;
                }
            }
            self.remove(worst, BAG_GONE_EVICTED, events);
        }
        let id = self.next_id;
        // Ids are for one shard's lifetime; wrapping past the wire's
        // field would collide, so the store saturates instead and the
        // last id is reused rather than aliasing an early one.
        self.next_id = self.next_id.saturating_add(1);
        let i = self.len;
        self.entries[i] = BackpackRec {
            id,
            qx,
            qy,
            qz,
            owner,
            expires: tick + bc.lifetime_ticks(items) as u64,
            items: *items,
        };
        self.len += 1;
        events.push(EV_BAG_DROPPED, id, owner, 0);
        Some(id)
    }

    /// Catch what an inventory could not hold. Merge into the nearest bag
    /// already standing in reach of `(qx, qz)` first, then stand a new one
    /// up for whatever still will not fit; `items` is left holding
    /// whatever nothing took. Returns the bag that ended up with the last
    /// of it, or `None` when nothing was caught.
    ///
    /// **`items` is cleared on the mint path too**, and that is a fix
    /// rather than a detail (merge-gate judge, pass -08, ranked fix 2).
    /// The sentence above was true of the merge and false of the mint:
    /// `stand_up` takes `&[ItemStack; INV_SLOTS]` and copies `*items`
    /// wholesale, so the buffer came back holding exactly what the new bag
    /// had just taken. Harmless while one caller owned one fresh buffer
    /// per player per tick — and this pass is the second caller, which is
    /// the arrangement that turns it into items duplicated into the world.
    /// A drain that runs twice, or a buffer reused across two verbs in one
    /// tick, is now safe by the contract instead of by luck.
    ///
    /// **The merge is what makes this bounded**, and it is the whole
    /// reason this is not just `stand_up`. A player swinging at a full
    /// pack pays a swing every `SWING_INTERVAL_TICKS` — roughly 47 a
    /// minute — and a bag per swing would churn `MAX_BACKPACKS` in five
    /// minutes of one player farming, evicting other people's death bags
    /// to do it. Merging first means standing still costs one bag however
    /// long you swing, and the eviction ladder keeps meaning what it says.
    ///
    /// The radius is `LOOT_REACH_M` — not a new knob, and the same arm
    /// that decides you may open a bag decides your spill can reach it.
    /// The pick is nearest-first with ties to the lower index, exactly
    /// `loot_nearest`'s rule, so it is a pure function of state.
    ///
    /// A bag that grows gets its expiry pushed out to what its new
    /// contents ask for and never pulled in — otherwise dropping a common
    /// item into a bag holding a rare one would shorten the rare one's
    /// clock, which is the ladder paying backwards.
    ///
    /// An inert ladder (`base_ticks == 0`) catches nothing and the
    /// overflow is destroyed exactly as it was before this lane existed —
    /// the same disarm `stand_up` and `drop_for` honour, so content that
    /// never armed the module still gets the pre-backpack world.
    #[allow(clippy::too_many_arguments)]
    pub fn spill_at(
        &mut self,
        bc: &BackpackContent,
        gc: &GatherContent,
        qx: i32,
        qy: i32,
        qz: i32,
        owner: u32,
        items: &mut [ItemStack; INV_SLOTS],
        tick: u64,
        events: &mut EventQueue,
    ) -> Option<u32> {
        if bc.base_ticks == 0 {
            return None; // inert content: the module is disarmed
        }
        if items.iter().all(|s| s.count == 0) {
            return None; // nothing to catch
        }
        let px = qx as f32 * POS_XZ_Q;
        let pz = qz as f32 * POS_XZ_Q;
        let mut best: Option<(f32, usize)> = None;
        for i in 0..self.len {
            let dx = self.entries[i].qx as f32 * POS_XZ_Q - px;
            let dz = self.entries[i].qz as f32 * POS_XZ_Q - pz;
            let d2 = dx * dx + dz * dz;
            if d2 > LOOT_REACH_M * LOOT_REACH_M {
                continue;
            }
            if best.is_none_or(|(bd2, _)| d2 < bd2) {
                best = Some((d2, i));
            }
        }
        if let Some((_, i)) = best {
            for stack in items.iter_mut() {
                if stack.count == 0 {
                    continue;
                }
                let cap = gc.stack_max_of(stack.item);
                let took = inv_add(&mut self.entries[i].items, stack.item, stack.count, cap);
                if took == 0 {
                    continue;
                }
                stack.count -= took;
                if stack.count == 0 {
                    stack.item = 0; // canonical empty
                }
            }
            let want = tick + bc.lifetime_ticks(&self.entries[i].items) as u64;
            if want > self.entries[i].expires {
                self.entries[i].expires = want;
            }
            if items.iter().all(|s| s.count == 0) {
                return Some(self.entries[i].id);
            }
        }
        let stood = self.stand_up(bc, qx, qy, qz, owner, items, tick, events);
        if stood.is_some() {
            // The new bag holds a copy of every stack in the buffer, so
            // the buffer is now a duplicate and not a remainder. `None`
            // deliberately leaves it alone: an inert ladder took nothing
            // and the caller destroying it is the pre-backpack world.
            *items = [ItemStack::default(); INV_SLOTS];
        }
        stood
    }

    /// Retire every bag whose timer ran out. One pass over the live
    /// entries (≤ `MAX_BACKPACKS`), taken every tick; the swap-remove is
    /// why the index does not advance on a hit.
    pub fn expire_due(&mut self, tick: u64, events: &mut EventQueue) {
        let mut i = 0;
        while i < self.len {
            if self.entries[i].expires <= tick {
                self.remove(i, BAG_GONE_DESPAWN, events);
            } else {
                i += 1;
            }
        }
    }

    /// Take everything that fits from the nearest bag within reach.
    /// Returns the looted bag's id when one was opened.
    ///
    /// The pick is nearest-in-reach, planar, exactly like the build/feed/
    /// door reach test — no id crosses the wire, so nothing here can be
    /// aimed at a bag the looter is not standing on. What does not fit
    /// stays in the bag, which is the whole reason a partly-looted bag
    /// keeps standing; a bag emptied by the take leaves immediately.
    ///
    /// Each moved slot announces itself with `EV_GATHER` — "these items
    /// entered your inventory", which is the event the client's `+N Item`
    /// toast already reads. Loot pays in the same currency gathering does,
    /// on purpose.
    /// Where the bag with this id sits in the dense store, or `None` if it
    /// is not there any more. The one lookup a client-named container goes
    /// through — a bag that despawned, emptied or was evicted while a
    /// panel was open resolves to `None`, and `inventory.rs` turns that
    /// into `REFUSE_M_NO_CONTAINER` rather than into a dropped session.
    /// Ids are never reused (`next_id` is monotonic and hashed), so this
    /// cannot answer with a different bag that took the same index.
    pub fn index_of_id(&self, id: u32) -> Option<usize> {
        if id == 0 {
            return None;
        }
        (0..self.len).find(|&i| self.entries[i].id == id)
    }

    /// Is this bag within arm's reach of that body? The same planar test
    /// `loot_nearest` makes, factored out rather than restated — one arm,
    /// one dequantize, one comparison, so the take-all verb and the
    /// per-slot verb can never disagree about what "in reach" means.
    ///
    /// Checked when the move resolves, never when a panel opened: reach is
    /// a fact about now, and a client that walked away is not consulted.
    pub fn in_reach(&self, i: usize, p: &Player) -> bool {
        if i >= self.len {
            return false;
        }
        let dx = self.entries[i].qx as f32 * POS_XZ_Q - p.body.qx as f32 * POS_XZ_Q;
        let dz = self.entries[i].qz as f32 * POS_XZ_Q - p.body.qz as f32 * POS_XZ_Q;
        dx * dx + dz * dz <= LOOT_REACH_M * LOOT_REACH_M
    }

    /// Read one slot of one bag. Out of range reads as empty — total, so a
    /// forged index is a refusal upstream and never an index panic here.
    pub fn slot(&self, i: usize, s: usize) -> ItemStack {
        if i >= self.len || s >= INV_SLOTS {
            return ItemStack::default();
        }
        self.entries[i].items[s]
    }

    /// Write one slot of one bag. Out of range writes nothing — the caller
    /// has already validated, and a silent no-op beats a panic on a path
    /// whose whole purpose is never to kill a session.
    pub fn set_slot(&mut self, i: usize, s: usize, stack: ItemStack) {
        if i >= self.len || s >= INV_SLOTS {
            return;
        }
        self.entries[i].items[s] = stack;
    }

    /// Retire a bag that a move emptied. Identical to the tail of
    /// `loot_nearest` and announced identically (`BAG_GONE_EMPTIED`), so a
    /// bag that ends empty leaves the world by one route whichever verb
    /// emptied it — and the wire's sync-walk restart contract stays keyed
    /// to one event rather than to which command was sent.
    pub fn drop_if_empty(&mut self, i: usize, events: &mut EventQueue) {
        if i < self.len && self.entries[i].is_empty() {
            self.remove(i, BAG_GONE_EMPTIED, events);
        }
    }

    pub fn loot_nearest(
        &mut self,
        gc: &GatherContent,
        p: &mut Player,
        events: &mut EventQueue,
    ) -> Option<u32> {
        let px = p.body.qx as f32 * POS_XZ_Q;
        let pz = p.body.qz as f32 * POS_XZ_Q;
        let mut best: Option<(f32, usize)> = None;
        for i in 0..self.len {
            let dx = self.entries[i].qx as f32 * POS_XZ_Q - px;
            let dz = self.entries[i].qz as f32 * POS_XZ_Q - pz;
            let d2 = dx * dx + dz * dz;
            if d2 > LOOT_REACH_M * LOOT_REACH_M {
                continue;
            }
            if best.is_none_or(|(bd2, _)| d2 < bd2) {
                best = Some((d2, i));
            }
        }
        let (_, i) = best?;
        let id = self.entries[i].id;
        for s in 0..INV_SLOTS {
            let stack = self.entries[i].items[s];
            if stack.count == 0 {
                continue;
            }
            let cap = gc.stack_max_of(stack.item);
            if cap == 0 {
                continue; // an item the ladder cannot stack cannot be taken
            }
            let took = inv_add(&mut p.inv, stack.item, stack.count, cap);
            if took == 0 {
                continue;
            }
            self.entries[i].items[s].count -= took;
            if self.entries[i].items[s].count == 0 {
                self.entries[i].items[s].item = 0; // canonical empty
            }
            events.push(
                EV_GATHER,
                p.id,
                ((stack.item as u32) << 16) | took as u32,
                0,
            );
        }
        if self.entries[i].is_empty() {
            self.remove(i, BAG_GONE_EMPTIED, events);
        }
        Some(id)
    }
}

impl Default for Backpacks {
    fn default() -> Self {
        Self::new()
    }
}

/// Vertical offset from the body position to where the bag rests, in the
/// y quantum. A body's position is its feet (`movement.rs`), so a bag
/// dropped verbatim sits on the ground already — this is zero and stated
/// rather than absent, so the client and the sim agree on it in one
/// place if it ever moves.
pub const BAG_Y_OFFSET_Q: i32 = 0;

#[cfg(test)]
mod tests {
    use super::*;

    fn stacks(rows: &[(u16, u16)]) -> [ItemStack; INV_SLOTS] {
        let mut inv = [ItemStack::default(); INV_SLOTS];
        for (i, &(item, count)) in rows.iter().enumerate() {
            inv[i] = ItemStack { item, count };
        }
        inv
    }

    /// A body at the origin carrying `rows` — everything `drop_for`
    /// reads off a player and nothing it does not.
    fn body(id: u32, rows: &[(u16, u16)]) -> Player {
        Player {
            id,
            active: true,
            inv: stacks(rows),
            ..Player::default()
        }
    }

    #[test]
    fn lifetime_is_the_rarest_thing_inside() {
        let bc = BackpackContent::probe_fixture();
        assert_eq!(
            bc.lifetime_ticks(&stacks(&[])),
            90,
            "an empty bag rides base"
        );
        assert_eq!(bc.lifetime_ticks(&stacks(&[(7, 5)])), 90);
        assert_eq!(
            bc.lifetime_ticks(&stacks(&[(7, 5), (2, 1)])),
            360,
            "one long-lived item raises the whole bag"
        );
    }

    #[test]
    fn an_inert_ladder_never_makes_a_bag() {
        let mut bags = Backpacks::new();
        let mut ev = EventQueue::default();
        assert_eq!(
            bags.drop_for(&BackpackContent::EMPTY, &body(7, &[(1, 5)]), 0, &mut ev),
            None
        );
        assert!(bags.is_empty());
        assert!(ev.is_empty());
    }

    #[test]
    fn an_empty_body_leaves_no_litter() {
        let mut bags = Backpacks::new();
        let mut ev = EventQueue::default();
        assert_eq!(
            bags.drop_for(&BackpackContent::probe_fixture(), &body(7, &[]), 0, &mut ev),
            None
        );
        assert!(bags.is_empty());
    }

    #[test]
    fn a_full_store_evicts_the_bag_nearest_its_own_despawn() {
        let bc = BackpackContent::probe_fixture();
        let mut bags = Backpacks::new();
        let mut ev = EventQueue::default();
        // Fill: bag k expires at 90 + k, so bag 0 is nearest its end.
        for k in 0..MAX_BACKPACKS {
            bags.drop_for(&bc, &body(1, &[(7, 1)]), k as u64, &mut ev)
                .expect("fill");
        }
        assert_eq!(bags.len(), MAX_BACKPACKS);
        let doomed = bags.entries()[0].id;
        let mut ev = EventQueue::default(); // only the overflow's own events
        let fresh = bags
            .drop_for(&bc, &body(2, &[(7, 1)]), 1_000, &mut ev)
            .expect("the death still drops");
        assert_eq!(bags.len(), MAX_BACKPACKS, "the cap held");
        assert!(bags.find(doomed).is_none(), "the nearest-gone bag left");
        assert!(bags.find(fresh).is_some());
        let codes: Vec<(u8, u32, u32)> = ev.entries().iter().map(|e| (e.code, e.a, e.b)).collect();
        assert_eq!(codes[0], (EV_BAG_REMOVED, doomed, BAG_GONE_EVICTED));
        assert_eq!(codes[1].0, EV_BAG_DROPPED);
    }

    #[test]
    fn ids_never_repeat_across_removals() {
        let bc = BackpackContent::probe_fixture();
        let mut bags = Backpacks::new();
        let mut ev = EventQueue::default();
        let a = bags.drop_for(&bc, &body(1, &[(7, 1)]), 0, &mut ev).unwrap();
        bags.expire_due(1_000, &mut ev);
        let b = bags
            .drop_for(&bc, &body(1, &[(7, 1)]), 1_000, &mut ev)
            .unwrap();
        assert_ne!(a, b, "a freed slot must not resurrect a retired id");
    }
}
