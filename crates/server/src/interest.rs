//! Class-S interest: which structural records a connection is owed, and
//! where it stands while deciding.
//!
//! `NETCODE.md` §7 asks for **one spatial truth** serving both replication
//! classes. Class D has had one since AOI v0 — a radial band with
//! hysteresis, `AOI_ENTER_CM` in and `AOI_EXIT_CM` out. Class S had none at
//! all: the join walk dripped the *entire* piece store to every client
//! regardless of distance, 32 records a tick, so at `MAX_PIECES` a joiner
//! spent 256 ticks (8.5 s) learning about every structure on the island
//! (`reference/NETWORK.md` §9.2.1 — the reference game's own 2014 mistake).
//!
//! This is the filter, expressed in the numbers class D already uses rather
//! than in new ones. Three parts:
//!
//! 1. **An anchor.** A client's walk is aimed from the position it was
//!    armed at, not from wherever the player is this tick. Fixing it is
//!    what makes the guarantee below a statement about a completed walk
//!    rather than about a moving target.
//! 2. **A radius**, [`PIECE_INTEREST_CM`] = `AOI_EXIT_CM`. The walk sends
//!    what is inside it and skips the rest.
//! 3. **A re-arm**, [`PIECE_REARM_CM`] = `AOI_EXIT_CM − AOI_ENTER_CM`.
//!    Move further than that from the anchor and the walk is re-armed at
//!    the new one. That difference is the class-D hysteresis band, spent
//!    here as the margin the walk streams *ahead* of what the client is
//!    promised.
//!
//! The guarantee those three buy, which `tests/piece_interest.rs` holds:
//!
//! > A client whose walk has completed holds every piece within
//! > `AOI_ENTER_CM` of its player.
//!
//! Because the anchor is never more than `PIECE_REARM_CM` from the player
//! (further re-arms), and `AOI_ENTER_CM + PIECE_REARM_CM = AOI_EXIT_CM` is
//! exactly what the walk streamed. The same predicate gates the
//! `EV_PIECE_PLACED` broadcast, so a piece placed *after* a walk finished
//! is covered by the identical arithmetic — one predicate, two callers, no
//! second opinion about where a client is.
//!
//! **What this is not.** It is not §7's chunk subscription: there is still
//! no grid, a re-arm re-walks the whole in-range set rather than the
//! difference, and nothing is ever un-subscribed — a client keeps what it
//! has been told (its mirror is capped at `MAX_PIECES`, the same bound the
//! server's store carries, so "keep" is bounded, not unbounded). What it
//! buys is the join cost and the raid cost, which is what §9.2.1 measured.

use sim_core::build::PieceRec;
use sim_core::limits::{AOI_ENTER_CM, AOI_EXIT_CM};

/// The build grid's cell pitch in centimetres — the units every AOI
/// comparison in this crate is already written in (`AOI_ENTER_CM`, and the
/// `qx * 3` that turns a 3 cm body quantum into one).
///
/// Written as an integer rather than derived from [`BUILD_CELL_M`] at
/// compile time on purpose: this whole module is integer arithmetic, so a
/// float ever appearing in it would be the one place a distance test could
/// answer differently on two builds. `cell_pitch_is_the_build_grid` below
/// holds the two equal, so moving `BUILD_CELL_M` reddens rather than
/// drifts.
pub const BUILD_CELL_CM: i64 = 300;

/// The radius the class-S walk streams at, in centimetres.
///
/// Not a knob: it is class D's *exit* radius, which is the point — the
/// grid `NETCODE.md` §7 wants is one grid, so the radius is one radius.
/// Class S takes the wider of the two bands because it has no per-tick
/// re-evaluation to fall back on; a walk is a promise kept once.
pub const PIECE_INTEREST_CM: i64 = AOI_EXIT_CM;

/// How far a player may travel from its walk's anchor before the walk is
/// re-armed at where it now stands, in centimetres.
///
/// Not a knob either: it is the class-D hysteresis band, `AOI_EXIT_CM −
/// AOI_ENTER_CM` (32 m), and it is what makes the module doc's guarantee
/// arithmetic instead of hopeful.
pub const PIECE_REARM_CM: i64 = AOI_EXIT_CM - AOI_ENTER_CM;

/// Piece-store entries a client's walk may **scan** in one tick, of which
/// at most `PIECE_SYNC_BATCH` are sent (`DECISIONS.md` §open, "class-S
/// interest v0").
///
/// A filtered walk has two rates where an unfiltered one had a single
/// number: what it looks at, and what it says. The batch stays the wire's
/// (`protocol`); this is wall 4's cap on the *work*, and it has a floor
/// that is not taste. The walk must be able to finish before a sprinting
/// player re-arms it, or it never converges — the same livelock the
/// removal restart used to cause, one mechanism over:
///
/// ```text
/// MAX_PIECES / B ticks × SPRINT_SPEED / TICK_HZ  <  PIECE_REARM_CM
/// 8192 / B × 5.5 / 30 m                          <  32 m     ⇒  B > 47
/// ```
///
/// 256 clears that by 5.4×: a full-store walk is 32 ticks (1.07 s), inside
/// which a sprinter covers 5.9 m of the 32 m budget. It costs 256 integer
/// distance tests per walking client per tick — nothing in steady state,
/// where every walk has finished and the scan does not run at all.
/// `tests/piece_interest.rs` holds the inequality against the constants it
/// is derived from.
pub const PIECE_SCAN_BATCH: usize = 256;

/// Squared planar distance between two centimetre positions.
///
/// `i64` throughout: the island is 2 km across, so a squared distance in
/// centimetres is ~4 × 10¹⁰ and overflows `i32` by four orders of
/// magnitude. Same shape as `update_interest`'s class-D compare.
#[inline]
pub fn d2_cm(a: (i64, i64), b: (i64, i64)) -> i64 {
    let dx = a.0 - b.0;
    let dz = a.1 - b.1;
    dx * dx + dz * dz
}

/// A body's quantized position as centimetres (`POS_XZ_Q` is 3 cm).
#[inline]
pub fn body_cm(qx: i32, qz: i32) -> (i64, i64) {
    (qx as i64 * 3, qz as i64 * 3)
}

/// The centre of build cell `(cx, cz)` in centimetres.
///
/// The *cell's* centre, not the piece's own anchor: a cell is 3 m across
/// and the radius is 208 m, so the two answers differ by at most 1% of the
/// radius — and taking the cell means every piece in a column answers the
/// filter identically, which is the property a chunk grid would give and
/// the one worth having early. A wall and the floor beside it are never
/// split across the boundary.
#[inline]
pub fn cell_cm(cx: u16, cz: u16) -> (i64, i64) {
    (
        cx as i64 * BUILD_CELL_CM + BUILD_CELL_CM / 2,
        cz as i64 * BUILD_CELL_CM + BUILD_CELL_CM / 2,
    )
}

/// Is the piece at grid address `(cx, cz)` inside the class-S interest of a
/// walk anchored at `anchor` (centimetres)?
#[inline]
pub fn piece_in_interest(anchor: (i64, i64), cx: u16, cz: u16) -> bool {
    d2_cm(cell_cm(cx, cz), anchor) <= PIECE_INTEREST_CM * PIECE_INTEREST_CM
}

/// [`piece_in_interest`] taking the record, for the walk's inner loop.
#[inline]
pub fn rec_in_interest(anchor: (i64, i64), rec: &PieceRec) -> bool {
    piece_in_interest(anchor, rec.cx, rec.cz)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_core::build::BUILD_CELL_M;

    /// The integer pitch is the build grid's, or every distance in this
    /// module is measured on a grid the world does not use.
    #[test]
    fn cell_pitch_is_the_build_grid() {
        assert_eq!(BUILD_CELL_CM, (BUILD_CELL_M * 100.0) as i64);
    }

    /// The two radii are the class-D band, not numbers of their own —
    /// `NETCODE.md` §7's "one spatial truth", asserted rather than
    /// asserted-in-prose.
    #[test]
    fn the_radii_are_class_ds_own_band() {
        assert_eq!(PIECE_INTEREST_CM, AOI_EXIT_CM);
        assert_eq!(PIECE_INTEREST_CM - PIECE_REARM_CM, AOI_ENTER_CM);
    }
}
