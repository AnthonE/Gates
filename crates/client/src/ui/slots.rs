//! The inventory grid, the container panel, and the move verb between them.
//!
//! ## Why this file is careful out of proportion to its size
//!
//! `CLAUDE.md`'s trap list names the item-move verb twice over. It is *the
//! most bug-prone thing in the reference* — three Oxide fixes in 28 minutes
//! on one 2019 day, the third titled as a fix of the fix, every one of them
//! a one-line splice-point move on move/stack/loot, and every one landing as
//! **the server disconnecting the client**, because a container state that
//! diverges reads as a forged request. And it is a **positional payload**,
//! the class where ~27 of Oxide's shipped corrections were the right value
//! in the wrong position and their own per-method hash gate caught none of
//! them.
//!
//! Two things follow, and both are structural rather than careful:
//!
//! 1. **[`MoveArgs`] has named fields and encodes itself.** The browser
//!    client marshals six positional `u32`s into a C ABI call and has to
//!    keep `MOVE_ARG_ORDER` and a JS smoke gate alive to state the order
//!    once (`web/src/invmove.js`). Nothing native needs that: the fields
//!    are named at construction, named at [`MoveArgs::encode`], and the
//!    compiler refuses a transposition between two of the four `u8`s the
//!    moment their names disagree. The whole failure class is closed by the
//!    type rather than watched by a gate.
//! 2. **Every refusal is checked before anything is marshalled**, in the
//!    order below, because the trap is validation ORDERING against the
//!    mutation. [`move_args`] returning `Some` *is* the proof they passed.
//!
//! ## Why the client refuses moves the encoder would carry
//!
//! `encode_action_move` bounds both slots against a flat `INV_SLOTS` and
//! lets box slot 20 encode; `sim_core::inventory::slots_in` bounds each slot
//! against **its own** container's width and answers `REFUSE_M_SLOT`. That
//! gap is deliberate on the wire's side — a tight check in the encoder would
//! make an over-wide slot a *frame* error, and a frame error ends the
//! session, which is the reference's disconnect-on-a-container-bug failure
//! exactly. So the encoder stays loose and the sim refuses politely, which
//! leaves the client owing the check: without it, a drop on box slot 20
//! encodes, crosses the wire, and comes back refused — a round trip and a
//! rolled-back prediction for a move this side could see was malformed.
//!
//! That is the quantize-both-sides law applied to containers: **the panel
//! must not draw a move it cannot address**, and it must send the values it
//! drew.

use protocol::{encode_action_move, WireError};
use sim_core::gather::ItemStack;
use sim_core::inventory::{CONT_BOX, CONT_MAX, CONT_SELF};
use sim_core::limits::{BOX_SLOTS, HOTBAR_SLOTS, INV_SLOTS};

/// Slots addressable in a container of `kind` — `sim_core::inventory`'s
/// `slots_in`, re-exported rather than mirrored so the two cannot drift.
pub use sim_core::inventory::slots_in;

/// Rows of six below the belt, so 6 + 24 = `INV_SLOTS`. The reference frame
/// (`Rust Images/inventory.jpeg`) is the same shape and for the same reason:
/// the belt is the row the world can see.
pub const GRID_COLS: usize = HOTBAR_SLOTS;
/// Rows in the main grid, below the belt.
pub const GRID_ROWS: usize = (INV_SLOTS - HOTBAR_SLOTS) / GRID_COLS;

const _: () = assert!(HOTBAR_SLOTS + GRID_ROWS * GRID_COLS == INV_SLOTS);

/// Where a slot sits on screen, in cells. Row 0 is the belt; rows 1.. are
/// the main grid. Pure geometry — the render layer multiplies by a cell
/// size and adds an origin, and nothing else knows the layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    pub col: usize,
    pub row: usize,
}

/// The cell a slot index draws in, or `None` past the container's width.
pub fn cell_of(kind: u8, slot: usize) -> Option<Cell> {
    if slot >= slots_in(kind) {
        return None;
    }
    Some(Cell {
        col: slot % GRID_COLS,
        row: slot / GRID_COLS,
    })
}

/// How much of a stack a drag carries. The reference's three gestures, and
/// the sim takes any `count` up to the stack, so all three are one verb with
/// different arithmetic rather than three verbs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Grab {
    /// Left-drag: the whole stack.
    #[default]
    All,
    /// Right-drag: half, rounded **up**, so a stack of one still moves
    /// rather than becoming a drag that silently does nothing.
    Half,
    /// Ctrl-drag: a single unit.
    One,
}

impl Grab {
    /// Units this gesture takes out of a stack of `held`. Zero in, zero out
    /// — an empty slot is refused by [`move_args`], not clamped here.
    pub fn units(self, held: u16) -> u16 {
        match self {
            Grab::All => held,
            Grab::Half => held.div_ceil(2),
            Grab::One => held.min(1),
        }
    }
}

/// A drag in flight: where it started and what it took.
///
/// Held by the render layer for exactly as long as the pointer is down. It
/// is **not** a prediction — the sim owns the containers and the panel
/// redraws from the next sync — so releasing over nothing simply drops it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Drag {
    pub kind: u8,
    pub slot: usize,
    pub grab: Grab,
    /// What the slot held when the pointer went down, so the label under
    /// the cursor is drawn from the same numbers the move will send.
    pub stack: ItemStack,
}

/// A validated move, ready for the wire. **Named fields, and the only
/// constructor is [`move_args`]** — a value of this type is the proof that
/// every refusal below was checked, in order, before anything was
/// marshalled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MoveArgs {
    /// The open container's handle — a bag id or a packed box address. Zero
    /// for a move inside your own inventory, and zero is **normalized here
    /// rather than trusted from the caller**: `world.rs` never reads the
    /// field for a self→self move and the encoder does not range-check it,
    /// so a stray handle would cross the wire and enter the WAL as a value
    /// nothing validates — which is where a wrong value lives forever.
    pub bag: u32,
    pub from_kind: u8,
    pub from_slot: u8,
    pub to_kind: u8,
    pub to_slot: u8,
    pub count: u16,
}

impl MoveArgs {
    /// Encode onto the reliable lane. The one call site of
    /// `encode_action_move` in the native client, so the argument order is
    /// written down exactly once and every field arrives by name.
    pub fn encode(&self, buf: &mut [u8]) -> Result<usize, WireError> {
        encode_action_move(
            self.bag,
            self.from_kind,
            self.from_slot,
            self.to_kind,
            self.to_slot,
            self.count,
            buf,
        )
    }
}

/// Marshal a drag into a move, or refuse it here rather than on the wire.
///
/// `inv` is the authoritative own-inventory view and `cont` the open
/// container's; **the count comes off whichever holds the source**, never
/// off the panel's label. The panel holds "Wood ×8" as a string and parsing
/// an 8 back out of it would be inventing the payload.
///
/// The refusals, in the order they are checked — the order is the point:
///
/// 1. **A kind past `CONT_MAX`.** The encoder range-checks it too, but a
///    refusal that has to cross the wire is a refusal the panel has already
///    drawn.
/// 2. **Two different ground containers.** The command carries ONE handle,
///    so bag→box is a destination the message cannot address and the sim
///    answers `REFUSE_M_NO_CONTAINER`. Same-kind passes: rearranging one
///    open box is box→box.
/// 3. **A slot past its own container's width**, by [`slots_in`] — see the
///    module note for why the encoder deliberately does not do this.
/// 4. **The same address twice.** The same slot *number* across two kinds
///    is a different address and is fine.
/// 5. **A ground end with a zero handle.** Not defensive tidying:
///    `deploy.rs`'s `box_index` has no zero guard and a box at cell (0,0)
///    level 0 packs to handle 0, so sending 0 for "no container known"
///    would move items in a stranger's box rather than being refused.
/// 6. **A count of zero, or more than the source holds.** The sim does not
///    clamp — a clamp is the silent divergence this verb exists to avoid —
///    so a count the client cannot back with a stack is refused here.
#[allow(clippy::too_many_arguments)]
pub fn move_args(
    bag: u32,
    from_kind: u8,
    from_slot: usize,
    to_kind: u8,
    to_slot: usize,
    grab: Grab,
    inv: &[ItemStack; INV_SLOTS],
    cont: &[ItemStack; INV_SLOTS],
) -> Option<MoveArgs> {
    // 1 · a kind the wire's two-bit field will carry.
    if from_kind > CONT_MAX || to_kind > CONT_MAX {
        return None;
    }
    // 2 · one ground container, or none. Which one is the handle's owner.
    let ground = if from_kind != CONT_SELF {
        from_kind
    } else {
        to_kind
    };
    if from_kind != CONT_SELF && to_kind != CONT_SELF && from_kind != to_kind {
        return None;
    }
    // 3 · each slot inside its OWN container's width.
    if from_slot >= slots_in(from_kind) || to_slot >= slots_in(to_kind) {
        return None;
    }
    // 4 · not the address it came from.
    if from_kind == to_kind && from_slot == to_slot {
        return None;
    }
    // 5 · the handle. Normalized to zero for self→self, required non-zero
    //     for anything on the ground.
    let handle = if ground == CONT_SELF { 0 } else { bag };
    if ground != CONT_SELF && handle == 0 {
        return None;
    }
    // 6 · the count, read from the source container's own view.
    let src = if from_kind == CONT_SELF { inv } else { cont };
    let held = src[from_slot].count;
    let count = grab.units(held);
    if count == 0 || count > held {
        return None;
    }

    Some(MoveArgs {
        bag: handle,
        // `try_from` rather than `as`: slot 3 above already bounded these
        // by `slots_in`, so a failure here is unreachable — and an `as`
        // truncation on an unreachable path is exactly how a wrong value
        // reaches a WAL nothing validates.
        from_kind,
        from_slot: u8::try_from(from_slot).ok()?,
        to_kind,
        to_slot: u8::try_from(to_slot).ok()?,
        count,
    })
}

/// Why a move bounced, for the panel's status line. `sim_core`'s
/// `REFUSE_M_*` in words; the numbers are the sim's and the sentences are
/// this side's.
pub fn refusal_text(reason: u8) -> &'static str {
    use sim_core::inventory::{
        REFUSE_M_COUNT, REFUSE_M_EMPTY, REFUSE_M_NO_CONTAINER, REFUSE_M_NO_ROOM, REFUSE_M_OVEN,
        REFUSE_M_REACH, REFUSE_M_SLOT, REFUSE_M_UNSTACKABLE,
    };
    match reason as u32 {
        REFUSE_M_SLOT => "that slot is not addressable",
        REFUSE_M_EMPTY => "there is nothing in that slot",
        REFUSE_M_COUNT => "that is more than the stack holds",
        REFUSE_M_NO_ROOM => "it does not fit there",
        REFUSE_M_NO_CONTAINER => "that container is gone",
        REFUSE_M_REACH => "too far away",
        REFUSE_M_UNSTACKABLE => "that item cannot be moved",
        REFUSE_M_OVEN => "a fire takes fuel and what it cooks",
        _ => "refused",
    }
}

/// The container panel's title. `CONT_SELF` has no panel, so it is named
/// for what it means rather than drawn.
pub fn container_title(kind: u8) -> &'static str {
    match kind {
        CONT_BOX => "BOX",
        CONT_SELF => "-",
        _ => "BAG",
    }
}

/// Width of the container panel's grid, in cells. A box is narrower than
/// the view that carries it: the wire ships `INV_SLOTS` slots whatever kind
/// is open, and the tail stays zero for a box — so a panel that drew all 30
/// would draw twelve slots and eighteen lies.
pub fn container_cols(kind: u8) -> usize {
    if kind == CONT_BOX {
        BOX_SLOTS.min(GRID_COLS)
    } else {
        GRID_COLS
    }
}
