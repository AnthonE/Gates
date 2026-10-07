//! Arrows that have stopped, until they lie on the ground
//! (`reference/PROJECTILES.md` §5; `NOW.md` §5 item 2).
//!
//! A stopped arrow ends up a **loose stack** (`grounditem.rs`): drawn,
//! named by `E`'s prompt and taken back by the payload-free
//! `Command::Pickup`, the same as a barrel's scatter. How it gets there:
//!
//!   * One that stopped on the world **sticks in it** where it went in —
//!     a trunk, a wall, the dirt — at the angle it flew, as Rust's does,
//!     the same tick. If what it is stuck in goes (a tree felled, a wall
//!     broken, a door opened) it falls (`World::unstick_arrows`).
//!   * One that ran out of flight in the air falls to the surface under it.
//!   * One that **dealt damage stands in the body it is in**, drawn where
//!     it went in, for the lodge (`content/balance.toml` `arrow_lodge_s`,
//!     Rust's five-minute despawn), then falls out at that body's feet.
//!     Anyone in reach may pull it out with `E` before then, the body's own
//!     player included (`World::pull_arrow`). It falls at once if its host
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
    /// The way it was flying when it **stuck**, max-norm quantized
    /// ([`stick_dir`]) — in the world (a trunk, a wall, the dirt) for an
    /// arrow in nothing, laid the tick it stops; or, for an arrow in a
    /// body, in that body's own frame ([`lodge_pose`]), so a client draws
    /// it standing out of the body at the angle it went in as the body
    /// turns. Zero is an arrow that falls: one out of flight, one that
    /// glanced.
    pub dir: [i8; 3],
    /// Where in its host it went in, centimetres from the host's feet in
    /// the host's own frame: x to its right, y up, z ahead ([`lodge_pose`]).
    /// Zero for an arrow in nothing.
    pub off: [i16; 3],
}

/// Where an arrow that met a body at `hit` (millimetres) went in, and
/// which way it was flying (`vel`, mm/tick), in that body's own frame: the
/// body stands at `feet` (millimetres) facing `yaw`, and its frame is the
/// client's (`render::bodies`) — x to its right, z ahead, so a drawn body's
/// transform puts the arrow back where it went in whichever way it turns.
pub fn lodge_pose(
    hit: (f32, f32, f32),
    feet: (i32, i32, i32),
    yaw: u16,
    vel: (i32, i32, i32),
) -> ([i16; 3], [i8; 3]) {
    let (fx, fz) = crate::yaw_lut::yaw_dir(yaw);
    let local = |x: f32, z: f32| (x * fz - z * fx, x * fx + z * fz);
    let (dx, dy, dz) = (
        hit.0 - feet.0 as f32,
        hit.1 - feet.1 as f32,
        hit.2 - feet.2 as f32,
    );
    let (lx, lz) = local(dx, dz);
    let cm = |v: f32| {
        crate::fmath::floor_i32(v / 10.0 + 0.5).clamp(i16::MIN as i32, i16::MAX as i32) as i16
    };
    let (vx, vz) = local(vel.0 as f32, vel.2 as f32);
    let dir = stick_dir(
        crate::fmath::floor_i32(vx),
        vel.1,
        crate::fmath::floor_i32(vz),
    );
    ([cm(lx), cm(dy), cm(lz)], dir)
}

/// A flight direction in a byte an axis: each component over the largest,
/// times 127. Integer division only, so it is the same bits on every
/// target; the reader normalizes. Zero only for a zero vector.
#[inline]
pub fn stick_dir(dx: i32, dy: i32, dz: i32) -> [i8; 3] {
    let m = dx
        .unsigned_abs()
        .max(dy.unsigned_abs())
        .max(dz.unsigned_abs()) as i64;
    if m == 0 {
        return [0; 3];
    }
    let q = |d: i32| (i64::from(d) * 127 / m) as i8;
    [q(dx), q(dy), q(dz)]
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
    /// Bumped on every lodge and every take — what the server's walk of
    /// the arrows in bodies keys on (`SUB_LODGED_SYNC`). Not state: it is
    /// neither hashed nor saved, and a reboot starting it again from zero
    /// costs a client one resend.
    stamp: u32,
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
            stamp: 0,
        }
    }

    /// The change counter the lodged-arrow walk keys on ([`Self::stamp`]'s
    /// field doc).
    #[inline]
    pub fn stamp(&self) -> u32 {
        self.stamp
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
        self.stamp = self.stamp.wrapping_add(1);
    }

    /// Add a stopped arrow. Never refuses: at capacity it evicts the entry
    /// with the smallest `ready_at` — the arrow due out soonest — and
    /// counts it (`MAX_SPENT_ARROWS`). Returns `true` if an eviction paid
    /// for this insert.
    pub fn lodge(&mut self, rec: SpentRec) -> bool {
        self.stamp = self.stamp.wrapping_add(1);
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
        self.stamp = self.stamp.wrapping_add(1);
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
/// `lay(round, x, y, z, dir)` is handed each arrow's round and the point
/// it stopped at, in millimetres, and the way it was flying if it stuck
/// there (`SpentRec::dir`, zero for one that falls); `World` sticks it or
/// finds the surface under it, and makes the loose stack. Returns how many
/// arrows were laid.
pub fn settle(
    spent: &mut SpentArrows,
    tick: u64,
    players: &[Player; MAX_PLAYERS],
    mobs: &Mobs,
    mut lay: impl FnMut(u16, i32, i32, i32, [i8; 3]),
) -> usize {
    let mut fell = 0usize;
    let mut i = 0;
    while i < spent.len {
        let rec = spent.entries[i];
        // Only an arrow in nothing can be stuck; one out of a body falls.
        let dir = if rec.host == 0 { rec.dir } else { [0; 3] };
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
                lay(rec.round, x, y, z, dir);
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
