//! Where every body stood, for the last `REWIND_TICKS` ticks.
//!
//! Lag compensation's store — slice 2 of
//! `findings/lagcomp-design-20260818.md` §7, landed on its own because a
//! store with a gate is checkable and a store plus a reader plus a wire
//! field is three things failing at once. **This line said "nothing reads
//! it yet" for one pass after `combat::strike` began to** (slice 4), which
//! is the dead-citation drift `CLAUDE.md` opens with, so: the readers are
//! `combat::strike` (melee) and `ranged::hitscan` (firearms), and the favour
//! that indexes both rides `Command::Input` (slice 3).
//!
//! `ranged::step` — an arrow already in the air — deliberately does **not**
//! read it, and that refusal is a type rather than a sentence
//! (`ranged::Pose`, `DECISIONS.md` §open).
//!
//! # What is stored, and why it is not a transform
//!
//! Both readers want the same three things off a victim: `qx`, `qy`, `qz`.
//! No yaw, no velocity, no `grounded` — `strike` is a planar distance plus a
//! cone, and the cone is the *attacker's* and stays present-tick; `hitscan`
//! is a planar closest-approach against a segment plus a height band off the
//! same cylinder, and the shooter's muzzle is likewise live. A player in this
//! tree has no orientable collider, so there is no transform to rewind; there
//! is a position.
//!
//! `id` is not padding. World slots are reused, and the server keeps a
//! `tracked_id` per slot for precisely this reason
//! (`server/src/core.rs`, which writes `0` for a slot with no live tenant —
//! the sentinel this file borrows). A row whose `id` disagrees with the
//! present tenant **must** fall back to the live body, or a rewind
//! resurrects somebody else's position under a stranger's name.
//!
//! # Wall 1 — the rewind is an integer number of ticks and nothing else
//!
//! There is no millisecond in this file, no float, no division and no map.
//! The index is `(tick & (REWIND_TICKS - 1))`, the lookup is an array index
//! in slot order, and the values handed back are the **same quanta the live
//! body holds** — a stored `i32` substituted for a live one, so the
//! dequantize downstream is byte-identical arithmetic on byte-identical
//! inputs. Native and wasm cannot diverge because nothing new is computed.
//!
//! # Wall 5 — the ring is derived, and the fallback is load-bearing
//!
//! **Not hashed.** It is derived from state that is already hashed, on the
//! precedent of `Pieces::cols` (*"Derived, never hashed"*) and the event
//! ring (*"derived output and stays out"*). Two shards agreeing on every
//! hash from tick 0 hold identical rings by construction.
//!
//! **Not saved**, on `worldsave.rs`'s arrows-in-flight precedent —
//! *"sub-second state whose whole meaning is a trajectory between two
//! ticks."* But unlike `cols` the ring is **not reconstructible at load**,
//! and that is the one place this could quietly break wall 5. What closes
//! it is the fallback: `pose_at` returns the live body whenever the row's
//! stamp is not the tick asked for, so a world loaded at tick `N` resolves
//! every strike at present until tick `N + REWIND_TICKS`. That is
//! deterministic given the *origin*, which is the sentence `worldsave.rs`
//! already widened wall 5 to — *same build + same origin + same command
//! stream → same state hashes*.
//!
//! **So do not "fix" the fallback into an extrapolation, a nearest-stamp
//! search or a saved ring.** Each of those makes a strike's outcome depend
//! on how the world arrived at tick `N`, which is the one thing wall 5
//! forbids. Said here because it reads like a rough edge and is the design.

use crate::limits::{MAX_PLAYERS, REWIND_MAX_TICKS, REWIND_TICKS};
use crate::movement::Body;
use crate::world::Player;

/// A slot with no live tenant. The same sentinel `server/src/core.rs` writes
/// into `tracked_id` for a dead slot, and the reason the id guard is safe:
/// a record left by an empty slot can never match a live player's id.
pub const NO_TENANT: u32 = 0;

/// A row that has never been written. Unreachable as a tick: at `TICK_HZ`
/// it is ~19 billion years, so a cold row can never be mistaken for a real
/// stamp and the ring needs no second "filled" array to say so.
const COLD: u64 = u64::MAX;

/// One body's position at the end of one tick — exactly 16 B, four `i32`s
/// wide with no padding.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RewindPose {
    /// The id of the player this position belonged to, or [`NO_TENANT`].
    pub id: u32,
    pub qx: i32,
    pub qy: i32,
    pub qz: i32,
}

impl RewindPose {
    /// The record an empty slot writes.
    pub const EMPTY: Self = Self {
        id: NO_TENANT,
        qx: 0,
        qy: 0,
        qz: 0,
    };

    /// The present-tick pose of a live body — what a caller hands `pose_at`
    /// as the fallback, and what `pose_at` returns whenever the ring cannot
    /// honestly answer.
    #[inline]
    #[must_use]
    pub fn live(id: u32, body: &Body) -> Self {
        Self {
            id,
            qx: body.qx,
            qy: body.qy,
            qz: body.qz,
        }
    }
}

/// The ring: `REWIND_TICKS` rows of `MAX_PLAYERS` poses, plus one tick stamp
/// per row.
///
/// **Boxed inside rather than boxed outside.** The design note proposed
/// `Box<Rewind>` on `World`; holding the rows in the box instead is the same
/// posture with a smaller `World` — `world_conts` already does exactly this
/// (*"Boxed inside, for `backpacks`' reason"*). `World` is built on the
/// stack by `ShardCore::new`, every wire test and `probe_parity`, and
/// wasm32's shadow stack has no guard page, so 12.8 kB belongs on the heap
/// however it is spelled. Nothing here allocates in the tick.
pub struct Rewind {
    /// `rows[tick & (REWIND_TICKS - 1)][slot]`.
    rows: Box<[[RewindPose; MAX_PLAYERS]; REWIND_TICKS]>,
    /// The tick each row was written for, [`COLD`] until it has been.
    stamps: [u64; REWIND_TICKS],
}

impl Rewind {
    #[must_use]
    pub fn new() -> Self {
        Self {
            rows: crate::boxed_array([RewindPose::EMPTY; MAX_PLAYERS]),
            stamps: [COLD; REWIND_TICKS],
        }
    }

    /// The row a tick lives in. A mask, never a modulo (wall 3).
    #[inline]
    const fn row_of(tick: u64) -> usize {
        (tick as usize) & (REWIND_TICKS - 1)
    }

    /// Record where every body stood at the end of tick `tick`.
    ///
    /// Called once, at the end of `World::tick`, immediately before the tick
    /// counter advances — so row `tick & (REWIND_TICKS - 1)` holds
    /// end-of-tick poses for `tick`, and during tick `T` the ring answers for
    /// `T-1 ..= T-REWIND_TICKS`.
    ///
    /// Sleepers are recorded like anyone else. `NETCODE.md` §8 excludes them
    /// because "they don't move"; in this tree they do, and the exclusion
    /// buys nothing anyway — the ring is a fixed array, so skipping a slot
    /// saves zero bytes and adds a branch to the hot write.
    pub fn write_row(&mut self, tick: u64, players: &[Player; MAX_PLAYERS]) {
        let r = Self::row_of(tick);
        let row = &mut self.rows[r];
        let mut i = 0;
        while i < MAX_PLAYERS {
            let p = &players[i];
            row[i] = if p.active {
                RewindPose::live(p.id, &p.body)
            } else {
                RewindPose::EMPTY
            };
            i += 1;
        }
        self.stamps[r] = tick;
    }

    /// Where slot `slot` stood `back` ticks before `tick`, or `live` when the
    /// ring cannot honestly answer.
    ///
    /// `back == 0` returns `live` **without touching the ring** — the row for
    /// the current tick has not been written yet, and this is also what makes
    /// a favour of zero bit-identical to the sim before rewinding existed.
    ///
    /// Four things fall back to `live`, and each is a correctness rule rather
    /// than a guard against a bug:
    ///
    /// - the tick asked for is before the world began (`checked_sub`);
    /// - the row's stamp is not that tick — it is cold, or it has been
    ///   overwritten, or `back` is past `REWIND_MAX_TICKS` and has aliased
    ///   onto a live row. **An out-of-range `back` needs no clamp here**: it
    ///   lands on a row stamped with a different tick and fails closed;
    ///   the failure of a forged favour is the shooter getting *less* help,
    ///   never more;
    /// - the slot held nobody at that tick ([`NO_TENANT`]);
    /// - the slot held **somebody else** — slots are reused and an id is
    ///   minted per connection, so this is the guard that stops a rewind
    ///   resurrecting a stranger's position.
    #[inline]
    #[must_use]
    pub fn pose_at(&self, tick: u64, slot: usize, back: u8, live: RewindPose) -> RewindPose {
        if back == 0 || slot >= MAX_PLAYERS {
            return live;
        }
        let Some(want) = tick.checked_sub(back as u64) else {
            return live;
        };
        let r = Self::row_of(want);
        if self.stamps[r] != want {
            return live;
        }
        let rec = self.rows[r][slot];
        if rec.id == NO_TENANT || rec.id != live.id {
            return live;
        }
        rec
    }

    /// The deepest rewind this ring will answer, in ticks — the clamp, not
    /// the row count. Exposed so a reader states the bound it is honouring
    /// instead of re-deriving it.
    #[inline]
    #[must_use]
    pub const fn max_back() -> u8 {
        REWIND_MAX_TICKS
    }
}

impl Default for Rewind {
    fn default() -> Self {
        Self::new()
    }
}
