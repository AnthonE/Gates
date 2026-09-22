//! The trust ledger's own ring: the one sim output a resync cannot repair
//! (`PLAYERS.md` wall 3, `NOW.md` §5d).
//!
//! Until this module, `EV_TRUST` rode the 256-seat drop-newest event ring
//! and nothing else. Every other passenger there is a fact about *state*,
//! which the join sync walk re-derives from the world; a trust row is a
//! fact about a *moment* (who worked whose door, and whether the owner was
//! home), so a dropped one was simply gone. This ring holds the same rows,
//! filled on the same line (`World::log_trust`), and sized so it cannot
//! overflow.
//!
//! ## Why it cannot overflow: the derivation, and what holds each link
//!
//! 1. **One row per command, held by the borrow checker.** A row reaches
//!    the ring only through [`TrustLedger::push`], which takes a
//!    [`TrustSeat`] *by value*, and a seat is neither `Clone` nor `Copy`. A
//!    verb that tried to log two rows for one command does not compile
//!    (`E0382`, use of a moved value). The doc test on [`TrustSeat`] goes red
//!    if anyone derives `Copy` on it.
//! 2. **One seat per applied command.** [`TrustLedger::seat`] is called once,
//!    inside `World::tick`'s command loop, which takes at most
//!    `MAX_COMMANDS_PER_TICK` commands. `tests/trust_ledger.rs` scrapes
//!    `src/` and fails on a second mint site.
//! 3. **Cleared every tick**, on `World::tick`'s first lines beside the
//!    event ring.
//!
//! So rows per tick ≤ seats per tick ≤ `MAX_COMMANDS_PER_TICK` ×
//! `TRUST_ROWS_PER_COMMAND`, which is `MAX_TRUST_ROWS_PER_TICK` by a
//! compile-time assert in `limits.rs`. The overflow branch still exists and
//! is **counted** ([`TrustLedger::overflow`]): a derivation is a claim, and
//! a counter is how a claim that stopped being true says so.
//!
//! ## What this is not
//!
//! - **Not hashed.** It is derived output, like the event ring: a pure
//!   function of commands and state that `state_hash` already covers.
//!   Hashing it would give one fact a second name.
//! - **Not saved.** The shard drains it after every tick
//!   (`server/src/trustlog.rs`), and the next tick clears it, so between
//!   ticks it holds nothing a save could lose. A save that carried it would
//!   hand the sink the same rows twice after a restore.
//! - **Not the end of `EV_TRUST`.** The event copy still rides the event
//!   ring, because the role checks and the tick-mate join in
//!   `tests/event_roles.rs` read it there. That copy is now the one allowed
//!   to drop. The shard reads this ring, never the event.

use crate::limits::MAX_TRUST_ROWS_PER_TICK;

/// One trust row: `actor` exercised `verb` against a record `counterparty`
/// owns, and `presence` says whether the counterparty was there.
///
/// `EV_TRUST`'s payload with its packed byte pair given names, because the
/// positional payload is where this class of code bleeds (`CLAUDE.md`'s
/// byte-golden entry). The event is built *from* this row
/// ([`TrustRow::packed`]), never beside it, so the two cannot disagree.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TrustRow {
    /// The player who acted.
    pub actor: u32,
    /// The player whose record the verb answered to.
    pub counterparty: u32,
    /// `world::TRUST_*`.
    pub verb: u8,
    /// `world::PRESENCE_*`.
    pub presence: u8,
}

impl TrustRow {
    /// `EV_TRUST.c`: `verb << 8 | presence`.
    pub const fn packed(&self) -> u32 {
        ((self.verb as u32) << 8) | self.presence as u32
    }
}

/// A command's right to **at most one** trust row.
///
/// Minted once per applied command by `World::tick`, moved into whichever
/// verb the command reaches, and spent by value in [`TrustLedger::push`].
/// It has a private field and no `Clone` or `Copy`, which is the whole of
/// link 1 in the module doc. A verb that wants two rows per command has to
/// change this type, and the ring's size in the same commit.
///
/// The path resolves and a seat can be passed along:
///
/// ```
/// fn pass(seat: sim_core::trust::TrustSeat) -> sim_core::trust::TrustSeat {
///     seat
/// }
/// ```
///
/// A seat cannot be spent twice. This is the red mutant: derive `Copy` on
/// the type and this block compiles, which fails the test.
///
/// ```compile_fail
/// fn twice(
///     seat: sim_core::trust::TrustSeat,
/// ) -> (sim_core::trust::TrustSeat, sim_core::trust::TrustSeat) {
///     (seat, seat)
/// }
/// ```
pub struct TrustSeat(());

/// This tick's trust rows. Cleared at tick start, drained by the shard after
/// the tick (`limits.rs`: `MAX_TRUST_ROWS_PER_TICK`).
///
/// The rows are boxed, the one-allocation-at-construction posture
/// `backpacks` and `arrows` take on a stack-built `World`, and built through
/// `boxed_array` so the array is never in a frame (`CLAUDE.md`'s wasm
/// shadow-stack trap). Nothing here allocates in the tick.
pub struct TrustLedger {
    rows: Box<[TrustRow; MAX_TRUST_ROWS_PER_TICK]>,
    len: usize,
    /// The tick these rows were applied in: `World::tick` read **before** its
    /// end-of-tick increment. The shard stamps the log from this and never
    /// from its own clock or from `world.tick` after the fact, which is one
    /// ahead by the time it looks.
    tick: u64,
    /// Seats minted since the last clear, one per applied command.
    seats: usize,
    /// Rows refused by a full ring since the last clear. Unreachable by the
    /// derivation above, and counted so that a broken derivation is loud.
    overflow: u32,
}

impl Default for TrustLedger {
    fn default() -> Self {
        Self::new()
    }
}

impl TrustLedger {
    pub fn new() -> Self {
        Self {
            rows: crate::boxed_array(TrustRow::default()),
            len: 0,
            tick: 0,
            seats: 0,
            overflow: 0,
        }
    }

    /// This tick's rows, in the order their commands were applied.
    pub fn rows(&self) -> &[TrustRow] {
        &self.rows[..self.len]
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The tick [`Self::rows`] were applied in.
    pub fn tick(&self) -> u64 {
        self.tick
    }

    /// Seats minted this tick: the number of commands the tick applied.
    pub fn seats(&self) -> usize {
        self.seats
    }

    /// Rows refused this tick. Zero unless the derivation broke.
    pub fn overflow(&self) -> u32 {
        self.overflow
    }

    /// Empty the ring for tick `tick`. Called by `World::tick` beside
    /// `EventQueue::clear`, before any command is applied.
    pub(crate) fn clear(&mut self, tick: u64) {
        self.len = 0;
        self.seats = 0;
        self.overflow = 0;
        self.tick = tick;
    }

    /// Mint one command's seat. **One call site** (`World::tick`'s command
    /// loop), and `tests/trust_ledger.rs` fails on a second.
    pub(crate) fn seat(&mut self) -> TrustSeat {
        self.seats += 1;
        TrustSeat(())
    }

    /// Record `row`, spending `seat`. The overflow branch is unreachable by
    /// the module's derivation; it counts rather than panics, because a
    /// panic in the tick would take the shard down over a diagnostic.
    pub(crate) fn push(&mut self, seat: TrustSeat, row: TrustRow) {
        let TrustSeat(()) = seat;
        if self.len == MAX_TRUST_ROWS_PER_TICK {
            self.overflow += 1;
            return;
        }
        self.rows[self.len] = row;
        self.len += 1;
    }
}
