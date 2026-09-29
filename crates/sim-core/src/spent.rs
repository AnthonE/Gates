//! Arrows that have stopped, until they lie on the ground
//! (`reference/PROJECTILES.md` §5; `NOW.md` §5 item 2).
//!
//! A stopped arrow ends up a **loose stack** (`grounditem.rs`): drawn,
//! named by `E`'s prompt and taken back by the payload-free
//! `Command::Pickup`, the same as a barrel's scatter. How it gets there:
//!
//!   * One that stopped on the world rests on the surface under where it
//!     stopped, the same tick.
//!   * One that ran out of flight in the air falls to the surface under it.
//!   * One that **dealt damage rides the body it is in** for the lodge
//!     (`content/balance.toml` `arrow_lodge_s`, theirs: 10 s), then falls
//!     out at that body's feet. The lodge is the reference's rule and the
//!     reason for it holds: an archer cannot re-collect the arrow they just
//!     shot someone with *during* the fight. It falls at once if its host
//!     dies, and where the host last stood if the host is gone.
//!   * ~15 % of landings break instead (`arrow_break_pct`), rolled by
//!     `ranged` at the stop.
//!
//! This store holds the arrows between the stop and the ground: the ones
//! in a body, and — inside one tick only — the ones `ranged` just stopped,
//! which [`settle`] lays down before the tick ends. Hashed and saved,
//! because an arrow in a body is state that will become an item.

use crate::limits::{MAX_PLAYERS, MAX_SPENT_ARROWS, MOB_ID_TAG};
use crate::mob::Mobs;
use crate::movement::{Body, POS_XZ_Q, POS_Y_Q};
use crate::world::Player;

/// One stopped arrow.
///
/// `round` is the **ammo** item, not the bow — the arrow you pull out of a
/// tree is the arrow you fired (`reference/PROJECTILES.md` §1 fact 5).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SpentRec {
    /// Millimetres, the arrow's own quanta (`ranged::Arrow`). Where it
    /// stopped; while it rides a body, that body's feet on the last tick
    /// the body stood.
    pub qx: i32,
    pub qy: i32,
    pub qz: i32,
    /// The round's item index — what the loose stack will be.
    pub round: u16,
    /// The tick a lodged arrow falls out of its host. Absolute, never
    /// decremented (`charge::ChargeRec::fires_at`'s rule).
    pub ready_at: u64,
    /// The body it is in: a player id, or `mob::mob_id(slot)`. Zero is an
    /// arrow in nothing, which rests this tick.
    pub host: u32,
    /// Which life of the host it went into — `Player::deaths` or
    /// `Mob::respawn_at` at the hit — so a host that died and came back
    /// does not carry it on.
    pub life: u64,
}

/// The stopped-arrow store — sim state, hashed and saved.
///
/// Dense and insertion-ordered, rewritten by swap-remove, like `Pieces`,
/// `Charges` and `WorldConts`. Boxed: `World` is built on the stack and
/// this is fixed capacity. One allocation at construction, none in the
/// tick.
#[derive(Clone, Debug)]
pub struct SpentArrows {
    entries: Box<[SpentRec; MAX_SPENT_ARROWS]>,
    len: usize,
    /// How many arrows this store has evicted to make room. Hashed, though
    /// it drives nothing: an eviction's only evidence is an absence.
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

    /// Add a stopped arrow. Never refuses: at capacity it evicts the entry
    /// with the smallest `ready_at` — the arrow due out soonest — and
    /// counts it (`MAX_SPENT_ARROWS`). Returns `true` if an eviction paid
    /// for this insert.
    pub fn lodge(&mut self, rec: SpentRec) -> bool {
        if self.len < MAX_SPENT_ARROWS {
            self.entries[self.len] = rec;
            self.len += 1;
            return false;
        }
        // Ties break on the lower index, which is deterministic because
        // the array's order is.
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

    /// Remove the arrow at `ix`, returning it. Swap-remove, so an index is
    /// invalidated by any removal.
    pub fn take_at(&mut self, ix: usize) -> Option<SpentRec> {
        if ix >= self.len {
            return None;
        }
        let rec = self.entries[ix];
        self.len -= 1;
        self.entries[ix] = self.entries[self.len];
        self.entries[self.len] = SpentRec::default();
        Some(rec)
    }
}

/// The `rng` channel the break roll draws on. 114, the next free one
/// after `mob.rs`'s think channel.
const CH_ARROW_BREAK: u32 = 114;

/// Does this landing break the arrow?
///
/// Keyed on `(seed, slot, tick)`, unique per landing because one arrow
/// slot retires at most once on a tick. Stateless, so a replay draws the
/// same bit. Multiply-shift rather than `% 100`, which is biased.
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

/// A body's feet in the arrow's millimetres.
#[inline]
pub fn feet_mm(b: &Body) -> (i32, i32, i32) {
    (
        b.qx * (POS_XZ_Q * MM_PER_M) as i32,
        b.qy * (POS_Y_Q * MM_PER_M) as i32,
        b.qz * (POS_XZ_Q * MM_PER_M) as i32,
    )
}

/// Where a lodged arrow's host is this tick: its feet if it still stands
/// in the life the arrow went into, `None` if it died, left or is gone.
fn host_feet(
    rec: &SpentRec,
    players: &[Player; MAX_PLAYERS],
    mobs: &Mobs,
) -> Option<(i32, i32, i32)> {
    if rec.host & MOB_ID_TAG != 0 {
        let m = mobs.m.get((rec.host & !MOB_ID_TAG) as usize)?;
        return (m.alive && m.respawn_at == rec.life).then(|| feet_mm(&m.body));
    }
    let p = players.iter().find(|p| p.active && p.id == rec.host)?;
    (!p.dead && u64::from(p.deaths) == rec.life).then(|| feet_mm(&p.body))
}

/// Lay down every arrow that is due, once a tick after the shots and the
/// deaths they caused: the ones that stopped on the world or in the air
/// this tick, and the lodged ones whose lodge ran out or whose host is
/// gone. A lodged arrow whose host still stands follows it.
///
/// `lay(round, x, y, z)` is handed each falling arrow's round and the
/// point it falls from, in millimetres; `World` finds the surface under it
/// and makes the loose stack. Returns how many arrows fell.
pub fn settle(
    spent: &mut SpentArrows,
    tick: u64,
    players: &[Player; MAX_PLAYERS],
    mobs: &Mobs,
    mut lay: impl FnMut(u16, i32, i32, i32),
) -> usize {
    let mut fell = 0usize;
    let mut i = 0;
    while i < spent.len {
        let rec = spent.entries[i];
        let from = if rec.host == 0 {
            Some((rec.qx, rec.qy, rec.qz))
        } else {
            match host_feet(&rec, players, mobs) {
                // Still in the body: follow it, and fall out at its feet
                // when the lodge runs out.
                Some(feet) if tick < rec.ready_at => {
                    let e = &mut spent.entries[i];
                    (e.qx, e.qy, e.qz) = feet;
                    None
                }
                Some(feet) => Some(feet),
                // Died or gone: where it last stood, which for a body that
                // died this tick is where the hit landed or where it fell.
                None => Some((rec.qx, rec.qy, rec.qz)),
            }
        };
        match from {
            Some((x, y, z)) => {
                spent.take_at(i);
                lay(rec.round, x, y, z);
                fell += 1;
            }
            None => i += 1,
        }
    }
    fell
}

/// Millimetres per metre — `ranged.rs`'s constant of the same name, which
/// is private to that module.
const MM_PER_M: f32 = 1000.0;
