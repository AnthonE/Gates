//! Arrows that have landed and can be picked up again
//! (`reference/PROJECTILES.md` §5 and §9.7, arrow recovery v0).
//!
//! **Why this store exists at all, and it is not comfort.** Our arrow was
//! spent permanently, so our ammunition was strictly harsher than the
//! reference's — and §9.6 refuses their bow damage number on exactly that
//! ground: theirs is priced against ammunition that comes back most of the
//! time, ours against ammunition that never came back. Taking their number
//! without their recovery loop is `BALANCE.md` §4.1's false-familiarity
//! trap, where the number matches and the weapon means something else. So
//! the loop has to land before the bow's numbers can track theirs at all.
//!
//! **All four of §9.7's pieces are here now, across two passes.** Pieces
//! 1 and 2 — the store and the lodge timer — landed at arrow recovery v0
//! with nothing able to reach them; pieces 3 and 4, the pickup verb
//! ([`pickup`]) and the `PROTO_VER` bump it costs, landed at v1 on wire
//! v53. `SpentArrows::take_near` was written and gated a pass ahead of its
//! only caller precisely so that the wire pass would be a wire pass, which
//! worked: the verb needed the store to grow one thing, a `peek_near` that
//! looks before it takes.
//!
//! **§9.7's one piece of advice was not followed, and it is worth saying
//! why rather than quietly.** It asks for this bump to ride *together
//! with* `EV_SHOT` (§9.2), because two wire bumps for one feature is two
//! sets of regenerated goldens and two chances to get wall 6 wrong. It
//! did not, because the blocker on `EV_SHOT` was never the bump — it was
//! a reading nobody had spoken: that event's payload is a muzzle speed and
//! a drop and the client re-flies exactly those integers, so a hitscan,
//! which has neither, needed a new event or a reading of its spare bit
//! patterns. Waiting for a decision nobody had made would have held the
//! operator's *"arrows come back"* behind it indefinitely. **The reading
//! landed one pass later** (wire v54, `ranged::hitscan`): `speed == 0` is
//! *instantaneous* and the low half becomes a reach. So the cost was
//! exactly the one extra bump this paragraph predicted, and no more.
//!
//! **Two rules, both theirs (§5).**
//!
//!   * ~15 % of landings break and are destroyed. The odds live in
//!     `content/balance.toml` (`arrow_break_pct`), never here.
//!   * An arrow that **dealt damage** may not be taken for 10 s; one that
//!     **missed** may be taken the moment it lands. That reads arbitrary
//!     and is not — it stops an archer re-collecting the arrow they just
//!     shot someone with *during* the fight, so a bow still runs dry in a
//!     sustained engagement while losing almost nothing to a day of
//!     hunting.
//!
//! **The clock is absolute, never a countdown.** `ready_at` is a tick
//! compared against `world.tick`, which is `charge::ChargeRec::fires_at`'s
//! rule and for its reason: a decremented counter is state a dropped tick
//! can corrupt, and a deadline cannot drift.
//!
//! **It is saved, and the arrows in flight are not.** `worldsave.rs` says
//! why it skips `Arrows` — "sub-second state whose whole meaning is a
//! trajectory between two ticks". A spent arrow is the opposite of that in
//! every respect: it is an item lying on a hillside with no velocity and
//! no deadline to expire on, and dropping it across a restart would delete
//! ammunition players had earned. Being in `state_hash` makes saving it
//! compulsory rather than merely right — a blob that dropped it would load
//! to a different hash than it was taken from, which is wall 5 failing at
//! the origin.

use crate::gather::{inv_add, GatherContent};
use crate::limits::MAX_SPENT_ARROWS;
use crate::movement::{POS_XZ_Q, POS_Y_Q};
use crate::world::{EventQueue, Player, EV_GATHER};

/// How far a player may reach for a landed arrow.
///
/// **Not a new knob — deliberately the same one**, which is
/// `LOOT_REACH_M`'s posture and `DRINK_REACH_M`'s, the third `pub use` of
/// `BUILD_REACH_M` rather than the first invented pickup distance. The
/// §open row that proposed this store said the reach "belongs beside
/// `BUILD_REACH_M`" and left it to the pass that gives the player a key;
/// beside it turned out to mean *it*. One argument for the alias over a
/// number: a player who can place a foundation at arm's length and loot a
/// backpack at arm's length has already been taught what arm's length is,
/// and a fourth radius would be a fourth thing to learn for no mechanic.
///
/// Measured in **3D**, unlike `LOOT_REACH_M` — [`SpentArrows::peek_near`]
/// compares `dy` as well, because an arrow lodged three metres up a trunk
/// is genuinely out of reach where a backpack at your feet never is.
pub use crate::build::BUILD_REACH_M as PICKUP_REACH_M;

/// `PICKUP_REACH_M` in the arrow store's own quanta.
///
/// The store is millimetres (`SpentRec::qx`) and the reach is metres, so
/// the conversion happens once, here, rather than at the call site where a
/// second caller would eventually get it wrong.
const PICKUP_REACH_MM: i32 = (PICKUP_REACH_M * 1000.0) as i32;

/// One arrow on the ground.
///
/// `round` is the **ammo** item, not the bow — the arrow you pull out of a
/// tree is the arrow you fired (`reference/PROJECTILES.md` §1 fact 5), so
/// a bow loaded with wooden arrows and firing them until they run out then
/// firing high-velocity ones gives back exactly what it spent, in the
/// right order. `Arrow::item` is the *weapon*, for the death screen, and
/// the two must not be confused: `Arrow` carries both for that reason.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SpentRec {
    /// Where it lies, millimetres — the arrow's own quanta, not the body's
    /// (`ranged::Arrow` says why they differ).
    pub qx: i32,
    pub qy: i32,
    pub qz: i32,
    /// The round's item index. What a pickup returns to the quiver.
    pub round: u16,
    /// The first tick this may be taken. Absolute, never decremented; the
    /// lodge is `ready_at - landed`, and a missed arrow's is zero so
    /// `ready_at` is simply the tick it landed.
    pub ready_at: u64,
}

/// The spent-arrow store — sim state, hashed and saved.
///
/// Dense and insertion-ordered, rewritten by swap-remove, exactly like
/// `Pieces`, `Charges` and `WorldConts`; the order is deterministic
/// because every insert and every removal is a tick's or a command's
/// consequence, replayed in the same order.
///
/// Boxed for `world_conts`' reason — `World` is built on the stack
/// (`ShardCore::new`, every wire test) and this is 12 kB of fixed
/// capacity. One allocation at construction, none in the tick.
#[derive(Clone, Debug)]
pub struct SpentArrows {
    entries: Box<[SpentRec; MAX_SPENT_ARROWS]>,
    len: usize,
    /// How many arrows this store has evicted to make room
    /// (`MAX_SPENT_ARROWS`'s stated policy). Hashed, though it drives
    /// nothing, for `World::evictions`' reason exactly: an eviction's only
    /// evidence is an *absence*, and two shards that evicted different
    /// arrows would hash the survivors identically for as long as nobody
    /// walked over the missing one. The counter makes that divergence loud
    /// on the tick it happens, and it is also the only way to tell whether
    /// the policy has ever fired or is guarding an unreachable case.
    evictions: u32,
}

impl Default for SpentArrows {
    fn default() -> Self {
        Self::new()
    }
}

impl SpentArrows {
    pub fn new() -> Self {
        Self {
            entries: crate::boxed_array(SpentRec::default()),
            len: 0,
            evictions: 0,
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[inline]
    pub fn entries(&self) -> &[SpentRec] {
        &self.entries[..self.len]
    }

    #[inline]
    pub fn evictions(&self) -> u32 {
        self.evictions
    }

    /// Replace the store from a decoded world save. Boot-only
    /// (`worldsave.rs`), like every other `restore` here.
    pub fn restore(&mut self, rows: &[SpentRec], evictions: u32) {
        let n = rows.len().min(MAX_SPENT_ARROWS);
        self.entries[..n].copy_from_slice(&rows[..n]);
        self.len = n;
        self.evictions = evictions;
    }

    /// Lay a landed arrow down. Never refuses: at capacity it evicts the
    /// entry with the smallest `ready_at` — the arrow that has been
    /// available to collect for longest and was not collected — which is
    /// `MAX_SPENT_ARROWS`'s stated policy and the argument for it.
    ///
    /// Returns `true` if an eviction paid for this insert.
    ///
    /// The scan is `MAX_SPENT_ARROWS` compares and no memmove, which is
    /// the reason the policy is expressed as a swap rather than as a shift
    /// of the whole array: a tick in which every one of `MAX_ARROWS`
    /// arrows lands at once would shift 128 × 12 kB under the other shape
    /// and compares 128 × 512 `u64`s under this one.
    pub fn lodge(&mut self, rec: SpentRec) -> bool {
        if self.len < MAX_SPENT_ARROWS {
            self.entries[self.len] = rec;
            self.len += 1;
            return false;
        }
        // Ties break on the lower index, which is deterministic because
        // the array's order is. `min_by_key` would too; the loop is here
        // because it states it.
        let mut worst = 0usize;
        for i in 1..self.len {
            if self.entries[i].ready_at < self.entries[worst].ready_at {
                worst = i;
            }
        }
        self.entries[worst] = rec;
        self.evictions = self.evictions.saturating_add(1);
        true
    }

    /// Take the nearest arrow that is ready and within `reach_mm` of
    /// `(qx, qy, qz)`, removing it. Returns the round's item index.
    ///
    /// **The reach is the caller's and not this module's**, deliberately.
    /// A pickup distance is the *verb's* knob — it belongs beside
    /// `BUILD_REACH_M` and `gather::REACH_M`, decided by the pass that
    /// gives the player a key to press (§9.7 piece 3). This function
    /// answers only "which arrow, if any", which is the half that can be
    /// gated before a wire exists.
    ///
    /// Nearest rather than first, for `reference/PROJECTILES.md` §7's
    /// reason applied one level out: the first entry in an array is an
    /// artefact of insertion order and the player is reaching for the one
    /// under their hand.
    pub fn take_near(
        &mut self,
        tick: u64,
        qx: i32,
        qy: i32,
        qz: i32,
        reach_mm: i32,
    ) -> Option<u16> {
        let (ix, _) = self.peek_near(tick, qx, qy, qz, reach_mm)?;
        self.take_at(ix)
    }

    /// Which arrow [`take_near`](Self::take_near) would take, and what it
    /// would return, **without taking it**.
    ///
    /// Split out of `take_near` on the pass that gave the player the key,
    /// because a pickup has to know what it is about to receive before it
    /// commits. The round is an item, the quiver has a cap, and a verb
    /// that removed the arrow and *then* discovered `inv_add` took nothing
    /// would have deleted ammunition the player earned — silently, since
    /// the arrow's only evidence is that it is lying there. `EV_GATHER`'s
    /// doc calls an unowed zero a lie; look-then-take is the shape that
    /// keeps the zero owed. `take_near` is retained on top of the two so
    /// the gates written before the verb still drive the same code.
    ///
    /// The index is into [`entries`](Self::entries) and is invalidated by
    /// any insert or removal, which is why the pair is used within one
    /// command and never held.
    pub fn peek_near(
        &self,
        tick: u64,
        qx: i32,
        qy: i32,
        qz: i32,
        reach_mm: i32,
    ) -> Option<(usize, u16)> {
        let reach = i64::from(reach_mm) * i64::from(reach_mm);
        let mut best: Option<(i64, usize)> = None;
        for i in 0..self.len {
            let e = self.entries[i];
            if tick < e.ready_at {
                continue;
            }
            // i64 throughout: two arrow coordinates are millimetres over a
            // 2 048 m island, so a squared separation overflows i32 at
            // 46 m and would wrap into a *near* answer.
            let (dx, dy, dz) = (
                i64::from(e.qx) - i64::from(qx),
                i64::from(e.qy) - i64::from(qy),
                i64::from(e.qz) - i64::from(qz),
            );
            let d2 = dx * dx + dy * dy + dz * dz;
            if d2 > reach {
                continue;
            }
            if best.is_none_or(|(bd, _)| d2 < bd) {
                best = Some((d2, i));
            }
        }
        let (_, ix) = best?;
        Some((ix, self.entries[ix].round))
    }

    /// Remove the arrow at `ix`, returning its round. `None` if `ix` is
    /// past the live length, which is the only way this can be asked a
    /// question it cannot answer.
    ///
    /// Swap-remove, the store's one removal shape — the same one
    /// `take_near` has always used, and the reason a `peek_near` index may
    /// not outlive the command that took it.
    pub fn take_at(&mut self, ix: usize) -> Option<u16> {
        if ix >= self.len {
            return None;
        }
        let round = self.entries[ix].round;
        self.len -= 1;
        self.entries[ix] = self.entries[self.len];
        self.entries[self.len] = SpentRec::default();
        Some(round)
    }
}

/// The `rng` channel the break roll draws on. 114, the next free one
/// after `mob.rs`'s think channel.
const CH_ARROW_BREAK: u32 = 114;

/// Does this landing break the arrow?
///
/// Keyed on `(seed, slot, tick)` — §9.7's own recipe — which is unique per
/// landing because one arrow slot retires at most once on a tick. Stateless,
/// so nothing is stored between ticks and a replay draws the same bit.
///
/// The draw is multiply-shift and not `% 100`, which is `loot.rs`'s form
/// and for its reason: modulo over a range that does not divide 2⁶⁴ is
/// biased, and a bias in the direction of "breaks" is a tax nobody wrote
/// down.
#[inline]
pub fn breaks(seed: u64, tick: u64, slot: usize, break_pct: u16) -> bool {
    if break_pct == 0 {
        return false;
    }
    if break_pct >= 100 {
        return true;
    }
    let h = crate::rng::cell_hash(seed, slot as i32, tick as i32, CH_ARROW_BREAK);
    (((h >> 32) * 100) >> 32) < u64::from(break_pct)
}

/// The pickup verb: take the nearest ready arrow within reach and put it
/// back in the quiver (`reference/PROJECTILES.md` §9.7 pieces 3 and 4).
///
/// **This is the caller `take_near` was written and gated for**, eighteen
/// days after it. Everything below it — the store, the lodge timer, the
/// break roll, the save section — landed with no way to reach it, because
/// §9.7 asked for the verb and its wire bump together and the bump is what
/// this pass spends.
///
/// **No target crosses the wire**, which is `ActionMsg::Loot`'s shape and
/// its argument verbatim: there is no id here to forge, no address to aim
/// past a wall, and no way to pick up an arrow the sender is not standing
/// on. The sim re-derives the pick from the sender's own body, so a client
/// that lies about where it is has to lie about it *to the movement code
/// first*, which is the only place we want that argument to happen.
///
/// **The pack being full leaves the arrow on the ground**, and says so.
/// `EV_GATHER`'s zero means exactly "the pack was full and every unit went
/// to the ground", and here the unit going to the ground is the unit
/// staying there — so the zero is owed rather than invented, and the
/// look-then-take split above is what makes it truthful. The alternative
/// shape, take-then-discover, deletes an arrow a player earned and reports
/// it as a pickup.
///
/// Returns the round that entered the quiver, or `None` if nothing did.
pub fn pickup(
    spent: &mut SpentArrows,
    gc: &GatherContent,
    tick: u64,
    p: &mut Player,
    events: &mut EventQueue,
) -> Option<u16> {
    // The player's body in the arrow's quanta. This is `ranged.rs`'s own
    // muzzle expression minus `ARROW_EYE_MM`: an arrow is picked up from
    // the body, not sighted from the eye, and the eye offset would tilt
    // the sphere upward by the height of a person.
    let qx = p.body.qx * (POS_XZ_Q * MM_PER_M) as i32;
    let qy = p.body.qy * (POS_Y_Q * MM_PER_M) as i32;
    let qz = p.body.qz * (POS_XZ_Q * MM_PER_M) as i32;

    let (ix, round) = spent.peek_near(tick, qx, qy, qz, PICKUP_REACH_MM)?;

    // An item the stack ladder cannot hold cannot be taken — `loot_nearest`
    // guards the same way, and the zero-ceiling case is `inv_add`'s stated
    // hazard rather than a hypothetical.
    let cap = gc.stack_max_of(round);
    if cap == 0 {
        return None;
    }
    let took = inv_add(&mut p.inv, round, 1, cap, gc.cond_max_of(round));
    if took == 0 {
        // Owed: an arrow was in reach and the quiver refused it. The arrow
        // is deliberately still lying there.
        events.push(EV_GATHER, p.id, (round as u32) << 16, 0);
        return None;
    }
    spent.take_at(ix);
    events.push(EV_GATHER, p.id, ((round as u32) << 16) | took as u32, 0);
    Some(round)
}

/// Millimetres per metre — `ranged.rs`'s constant of the same name, which
/// is private to that module. Restated rather than exported because the
/// two uses are the two halves of one round trip (an arrow leaves the body
/// in `ranged`, and comes back to it here) and a shared `pub` constant for
/// the number 1000 buys nothing.
const MM_PER_M: f32 = 1000.0;
