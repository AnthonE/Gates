//! The move verb: one item, one slot, one container to another.
//!
//! `CLAUDE.md`'s trap list calls this the most bug-prone thing in the
//! reference and names the mechanism exactly: three Oxide fixes in
//! twenty-eight minutes on one 2019 day, the third titled as a fix of the
//! fix, all one-line splice-point moves on move/stack/loot, and **all
//! three landed as the server disconnecting the client** — because
//! container state diverged and a request computed against a container the
//! server does not agree about reads as a forged one. The bug is
//! validation ordering against the mutation, never arithmetic.
//!
//! So this module is built so that ordering cannot be gotten wrong by
//! editing it. Validation does not *precede* mutation by discipline; it is
//! a **different function that cannot mutate**, because it is handed
//! `ItemStack` by value and returns a plan:
//!
//! ```text
//!   plan_move(cap, src, dst, count) -> Result<MovePlan, u32>   // reads copies
//!   resolve(plan, src, dst)         -> (ItemStack, ItemStack)  // pure
//! ```
//!
//! Neither takes a `&mut` to anything. A caller physically cannot write a
//! half-applied move, and a later edit that wants to cannot do it without
//! changing a signature — which is a thing a reviewer sees. The only
//! writes live in `World::move_item` (`world.rs`), after both calls have
//! returned, and there are exactly two of them.
//!
//! ## Every ambiguity is a refusal, and that is the quantize-both-sides law
//!
//! `CLAUDE.md`: *quantize both sides or prediction drifts by rounding —
//! the server sims on the values it transmits.* Applied to containers, the
//! rounding is **partial application**. A client that asks to move 40 and
//! gets 25 because that is all that fit has no way to notice: it drew 40,
//! the sim did 25, and the two states differ by fifteen forever. So no
//! move here ever does *part* of what was asked. It moves exactly `count`
//! or it moves nothing and says why. `inv_add`'s take-what-fits behaviour
//! is right for a pickup, where the client asked for no particular number;
//! it is wrong for a verb the client has already drawn.
//!
//! The other half of that law is the refusal *payload*: `EV_MOVE_REFUSED`
//! echoes the address the sender asked for, so the client rolls back the
//! exact move it predicted rather than resyncing a whole container.
//!
//! And nothing here ever disconnects. Every reachable input — a forged
//! slot, a container that despawned mid-drag, a bag out of reach, a count
//! past the stack — lands on an announced refusal, because the failure
//! mode being defended against *is the disconnect*.

use crate::gather::ItemStack;
use crate::limits::{BOX_SLOTS, INV_SLOTS};

/// The sender's own inventory: `Player::inv`, all `INV_SLOTS` of it. The
/// hotbar is slots `0..HOTBAR_SLOTS` of the same array (`world.rs`), so
/// "arrange the hotbar" and "arrange the backpack" are one verb here, not
/// two — which is also why the first thing this unlocks is choosing what
/// you are holding.
pub const CONT_SELF: u8 = 0;
/// A backpack on the ground, named by the `bag` field of the command
/// (`backpack.rs`). Reach is checked at the moment the move resolves, not
/// at the moment a panel opened: a bag you walked away from is a bag you
/// cannot take from, and the sim never trusts that the client noticed.
pub const CONT_BAG: u8 = 1;
/// A deployed storage box (`ARCH_BOX`, `deploy.rs`), named by the same
/// handle field as a bag — but carrying a packed **address**, not an id:
/// `box_key(cx, cz, level)`. A box is furniture, so it has no identity to
/// hand out; the client already receives `cx`/`cz`/`level` in the deploy
/// sync it draws the box from, and can therefore name one without a byte
/// of new wire.
///
/// Open to anyone who can reach it, exactly like an unlocked door
/// (`use_door`) and exactly like a bag. Not owner-only: a box a raider
/// cannot empty is the "a raid takes nothing" complaint with an extra
/// step. Lock v0 is the spoken decision that would change this
/// (DECISIONS.md §open) — when it lands it belongs on the same `locked`
/// bit a door already carries, not on a second rule invented here.
pub const CONT_BOX: u8 = 2;
/// The highest kind the sim understands. Two bits cross the wire, so
/// value 3 is forgeable and refuses — at decode as `Malformed`
/// (`protocol`), and here as `REFUSE_M_SLOT` for a command that never
/// crossed a wire at all (a bot, a replayed WAL).
pub const CONT_MAX: u8 = CONT_BOX;

/// Slots addressable in a container of `kind`. A box is smaller than an
/// inventory (`BOX_SLOTS` against `INV_SLOTS`), so the address check is
/// per-kind rather than one `INV_SLOTS` bound for everything — otherwise
/// slots 12..30 of a box would be nameable, land inside the record, and
/// be invisible to every reader that stops at `BOX_SLOTS`.
pub fn slots_in(kind: u8) -> usize {
    match kind {
        CONT_BOX => BOX_SLOTS,
        _ => INV_SLOTS,
    }
}

/// Why a move was refused — the `b` field of `EV_MOVE_REFUSED`. Zero is
/// reserved as "no reason", so the encoder can refuse it the way
/// `encode_event_consume_refused` already refuses a zero reason.
///
/// A slot index outside the container, a kind past `CONT_MAX`, or a move
/// onto the slot it came from. All three are "that address is not a thing
/// you can name", and the client's fix for each is the same: it desynced,
/// redraw from the next sync.
pub const REFUSE_M_SLOT: u32 = 1;
/// The source slot holds nothing. Distinct from `REFUSE_M_COUNT` on
/// purpose: an empty source means the client's picture of the container is
/// stale, a bad count means only the drag was.
pub const REFUSE_M_EMPTY: u32 = 2;
/// `count` is zero, or more than the source slot actually holds. **Not
/// clamped** — see the module note: a clamp is the silent divergence this
/// verb exists to avoid.
pub const REFUSE_M_COUNT: u32 = 3;
/// The destination cannot take the whole move: a partial merge, or a
/// partial stack dropped onto a different item. Both are legal asks in a
/// UI and neither is a legal *result*, so the client redraws and the
/// player drags again.
pub const REFUSE_M_NO_ROOM: u32 = 4;
/// The named container is not in the store — a bag that despawned, was
/// emptied or was evicted while the panel was open, or a box address with
/// no box standing at it. The most ordinary refusal there is, and
/// historically the one that got sent as a disconnect.
///
/// It is also what a move **between two ground containers** gets, and
/// that is a wire fact rather than a rule: the command carries one
/// container handle for both sides, so `CONT_BAG` → `CONT_BOX` names a
/// destination the message has no room to address. One of the two named
/// containers genuinely has no address, which is this reason exactly. A
/// second handle is a field the action does not have and an eighth reason
/// is a width `REFUSE_M_BITS` does not have, so the honest answer is to
/// refuse it and let the player pass the item through their own hands —
/// which is the move they would make with one panel open anyway.
pub const REFUSE_M_NO_CONTAINER: u32 = 5;
/// The container is real and out of arm's reach. Reach is planar against
/// the container's own position, measured when the move resolves and
/// never when a panel opened.
pub const REFUSE_M_REACH: u32 = 6;
/// The item has no stack ladder (`GatherContent::stack_max` is zero for
/// it, which is also what an item index past the table reads as). Nothing
/// the ladder cannot size can be moved, for the same reason
/// `loot_nearest` already skips it.
pub const REFUSE_M_UNSTACKABLE: u32 = 7;
/// The largest reason above. Three bits hold `1..=7`, which is what the
/// wire spends, and a reason added past this needs the width to move.
pub const REFUSE_M_MAX: u32 = REFUSE_M_UNSTACKABLE;

/// What a validated move will do. Constructed only by `plan_move`, so a
/// value of this type *is* the proof that every check passed — the
/// ordering trap closed by the type system rather than by reading order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MovePlan {
    /// `count` of `item` leaves the source slot for the destination,
    /// which was either empty or already holding the same item with room.
    Transfer { item: u16, count: u16 },
    /// Two occupied slots holding different items exchange wholesale.
    /// Only ever planned for a whole-stack ask — a partial drop onto a
    /// different item is `REFUSE_M_NO_ROOM`, because there is no third
    /// slot to put the remainder in and inventing one is how a dupe ships.
    Swap,
}

/// Decide what a move does, or why it does not. Pure: it is handed copies
/// and it returns a plan, so it has nothing to corrupt.
///
/// `cap` is the stack ceiling for the **source** item
/// (`GatherContent::stack_max_of`). The destination's ceiling is not
/// consulted and does not need to be: a merge is same-item so it is the
/// same ceiling, and a swap moves two stacks that were already inside
/// their own ceilings into each other's slots without changing either
/// count.
///
/// The checks are ordered from "your address is nonsense" outward to "your
/// arithmetic does not fit", so the reason the client hears is the most
/// specific true one.
pub fn plan_move(cap: u16, src: ItemStack, dst: ItemStack, count: u16) -> Result<MovePlan, u32> {
    if src.count == 0 {
        return Err(REFUSE_M_EMPTY);
    }
    if count == 0 || count > src.count {
        return Err(REFUSE_M_COUNT);
    }
    if cap == 0 {
        return Err(REFUSE_M_UNSTACKABLE);
    }
    // A stack larger than its own ladder cannot be built by any move, even
    // when the source somehow already holds one (a content edit between
    // sessions can do that). Checked before the destination branch so the
    // ceiling holds for the empty-slot case too.
    if count > cap {
        return Err(REFUSE_M_NO_ROOM);
    }
    if dst.count == 0 {
        return Ok(MovePlan::Transfer {
            item: src.item,
            count,
        });
    }
    if dst.item == src.item {
        // `saturating_sub`: a destination already over its ladder yields
        // zero room and refuses, rather than wrapping into a huge one.
        let room = cap.saturating_sub(dst.count);
        if count > room {
            return Err(REFUSE_M_NO_ROOM);
        }
        return Ok(MovePlan::Transfer {
            item: src.item,
            count,
        });
    }
    if count != src.count {
        return Err(REFUSE_M_NO_ROOM);
    }
    Ok(MovePlan::Swap)
}

/// Turn a plan into the two slots that replace the ones it was planned
/// from. Returns `(new_source, new_destination)`.
///
/// Pure, and total by construction — every arithmetic step below is
/// underwritten by a check `plan_move` already made, so there is nothing
/// here that can fail and no path that writes one slot without the other.
/// The caller writes both or neither.
///
/// Emptied slots zero **both** fields, which is `gather.rs`'s canonical-
/// empty rule and therefore a `state_hash` requirement: two replays that
/// disagree about the item id left behind in a slot holding zero of it
/// would hash differently while being the same world.
pub fn resolve(plan: MovePlan, src: ItemStack, dst: ItemStack) -> (ItemStack, ItemStack) {
    match plan {
        MovePlan::Swap => (dst, src),
        MovePlan::Transfer { item, count } => {
            let left = src.count - count;
            let new_src = if left == 0 {
                ItemStack::default()
            } else {
                ItemStack {
                    item: src.item,
                    count: left,
                }
            };
            let new_dst = ItemStack {
                item,
                count: dst.count + count,
            };
            (new_src, new_dst)
        }
    }
}

/// Why a container panel stopped being served — the `why` of
/// `cont_closed` (wire v19). A close is never silent: whatever ends a
/// subscription says which of these it was, because "it is gone" and
/// "walk back" are different sentences to a player and the client would
/// otherwise have to guess by watching a panel go stale.
///
/// The player asked. The only reason a client already knows, sent anyway
/// so that exactly one code path closes a panel.
pub const CLOSE_ASKED: u8 = 0;
/// The container left the world — a bag looted empty or expired, a box
/// broken open. `REFUSE_M_NO_CONTAINER`'s sentence, on the read side.
pub const CLOSE_GONE: u8 = 1;
/// The container is real and out of arm's reach. Measured the same way
/// `REFUSE_M_REACH` measures it and against the same containers, so a
/// panel closes on exactly the step that would have started refusing
/// moves — the container half of the quantize-both-sides law: the client
/// must never be looking at a container whose moves the sim would now
/// decline.
pub const CLOSE_REACH: u8 = 2;
/// The largest reason above. Two bits hold `0..=3`, which is what the
/// wire spends; a fourth reason is free and a fifth needs the width.
pub const CLOSE_MAX: u8 = CLOSE_REACH;

/// Pack a move's address into one `u32` for the event lane:
/// `from_kind << 24 | from_slot << 16 | to_kind << 8 | to_slot`.
///
/// One field, four parts, and the parts are kept in the order the verb is
/// spoken in — from, then to — so a transposed pair reads as a move in the
/// opposite direction rather than as a different slot. `test_event_roles`
/// asserts the four apart from each other for exactly that reason.
pub fn addr(from_kind: u8, from_slot: u8, to_kind: u8, to_slot: u8) -> u32 {
    ((from_kind as u32) << 24)
        | ((from_slot as u32) << 16)
        | ((to_kind as u32) << 8)
        | (to_slot as u32)
}
