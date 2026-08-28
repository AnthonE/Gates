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
//! **What landed here and what did not.** §9.7 decomposes recovery into
//! four pieces and says only the first is small. This module is pieces 1
//! and 2 — the store and the lodge timer. Pieces 3 and 4 are the pickup
//! verb and the `PROTO_VER` bump it needs, and §9.7 asks for them
//! *together with* `EV_SHOT` (§9.2) rather than after it, because two wire
//! bumps for one feature is two sets of regenerated goldens and two
//! chances to get wall 6 wrong. **So nothing can pick one of these up
//! yet.** [`SpentArrows::take_near`] is the whole of what the verb will
//! call, and it is written and gated here so that the wire pass is a wire
//! pass.
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

use crate::limits::MAX_SPENT_ARROWS;

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
