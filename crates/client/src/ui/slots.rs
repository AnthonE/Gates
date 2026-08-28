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

use protocol::event::ItemCatalog;
use protocol::{encode_action_move, WireError};
use sim_core::combat::{ARMOR_MAX_PCT, WEAR_BODY, WEAR_HEAD};
use sim_core::deploy::{box_key, DeployContent, DeployRec, ARCH_BOX};
use sim_core::gather::ItemStack;
use sim_core::inventory::{is_own, CONT_BOX, CONT_MAX, CONT_SELF, CONT_WEAR, CONT_WORLD};
use sim_core::limits::{HOTBAR_SLOTS, INV_SLOTS, WEAR_SLOTS};

/// Slots addressable in a container of `kind` — `sim_core::inventory`'s
/// `slots_in`, re-exported rather than mirrored so the two cannot drift.
pub use sim_core::inventory::slots_in;

/// Rows of six below the belt, so 6 + 24 = `INV_SLOTS`. The reference frame
/// (the reference `inventory.jpeg`) is the same shape and for the same reason:
/// the belt is the row the world can see.
pub const GRID_COLS: usize = HOTBAR_SLOTS;
/// Rows in the main grid, below the belt.
pub const GRID_ROWS: usize = (INV_SLOTS - HOTBAR_SLOTS) / GRID_COLS;

const _: () = assert!(HOTBAR_SLOTS + GRID_ROWS * GRID_COLS == INV_SLOTS);

/// The hotbar slot a scroll of `notches` lands on, wrapping at both ends.
///
/// Pure, and here rather than inside `render::input`, for this module's own
/// stated reason: a mapping that lives inside a system is a mapping no test
/// in the code tier can call.
///
/// `notches` is signed, and **negative walks toward slot 1** — a wheel pushed
/// away from the player. That is the direction every reference this client's
/// players arrive from uses, and it is the half of a scroll binding that is
/// actually worth gating, because it is invisible in a screenshot and wrong
/// in exactly one of two ways.
///
/// **It wraps rather than saturating.** Six slots under a wheel is a ring;
/// saturating would make the wheel unable to reach slot 6 from slot 1 without
/// a full sweep back, which is the one thing a wheel is for.
pub fn hotbar_scrolled(sel: u8, notches: i32) -> u8 {
    (sel as i32)
        .wrapping_add(notches)
        .rem_euclid(HOTBAR_SLOTS as i32) as u8
}

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
    //
    //     `is_own`, not `!= CONT_SELF`, and it is `sim_core`'s predicate
    //     rather than a second one written here — the same reason
    //     `slots_in` is re-exported above. A player carries two containers
    //     since armor v1, and both of them are addressed without a handle.
    let ground = if !is_own(from_kind) {
        from_kind
    } else {
        to_kind
    };
    if !is_own(from_kind) && !is_own(to_kind) && from_kind != to_kind {
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
    // 5 · the handle. Normalized to zero when both sides are on the
    //     player, required non-zero for anything on the ground.
    //
    //     **This check is why the wear panel would have looked broken
    //     rather than refused.** Under `ground != CONT_SELF` a
    //     `CONT_SELF → CONT_WEAR` move takes `ground == CONT_WEAR`, has no
    //     handle to offer, and returns `None` here — which sends nothing
    //     at all, so the drag snaps back with no refusal, no event and no
    //     toast. Worse, with a box open it would have shipped the *box's*
    //     `box_key` as a wear move's handle.
    let handle = if is_own(ground) { 0 } else { bag };
    if !is_own(ground) && handle == 0 {
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
        REFUSE_M_REACH, REFUSE_M_SLOT, REFUSE_M_UNSTACKABLE, REFUSE_M_WEAR,
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
        // The reason armor v1 minted, unwired until v52: a helmet dragged
        // onto the body slot, or anything that is not armor dragged onto
        // either, bounced with the generic word. The client refuses most
        // of these before the round trip now (`wearable_here`), so what
        // reaches this arm is the disagreement — a stale catalog, or a
        // slot the server knows about and this build does not.
        REFUSE_M_WEAR => "that is not what goes in that slot",
        _ => "refused",
    }
}

/// The protection a worn set is worth, whole percent — the client's read
/// of `sim_core::combat::worn_pct`, off the catalog columns wire v52
/// added.
///
/// **The same arithmetic, deliberately, and gated against the original**
/// (`tests/ui.rs` §H). Both sum every slot rather than the slot that was
/// hit, both pay a piece only in the slot its baked row names — the check
/// is against the *row*, not against where the stack is sitting — and both
/// clamp to `combat::ARMOR_MAX_PCT`. A second implementation is what this
/// is, and it is the shape `CLAUDE.md` warns about; what makes it safe is
/// that the gate drives the sim's function and this one over the same
/// worn sets and compares, rather than rebuilding this one's body.
///
/// `worn` is the client's view of the `CONT_WEAR` container, which is
/// `INV_SLOTS` wide on the wire with a tail of zeros — the loop reads the
/// sim's own `WEAR_SLOTS` and nothing past it, so a smuggled stack in slot
/// 7 protects nobody here either.
pub fn worn_pct(catalog: &ItemCatalog, worn: &[ItemStack; INV_SLOTS]) -> u32 {
    let mut pct = 0u32;
    for (i, stack) in worn.iter().enumerate().take(WEAR_SLOTS) {
        if stack.count > 0 && catalog.wear_slot(stack.item as usize) as usize == i + 1 {
            pct += catalog.armor_pct(stack.item as usize) as u32;
        }
    }
    pct.min(ARMOR_MAX_PCT)
}

/// May this item be worn in wear-container slot `s`? The client's read of
/// `sim_core::combat::wearable_in`, off the same catalog column.
///
/// This is what lets a wrong-slot drag be refused **before** the round
/// trip, on the sim's own predicate rather than a second list of what
/// counts as armor. `s` is a container slot index and is bounded here
/// rather than at the caller: `s + 1` on a `u8` wraps to 0 in release and
/// `0` is `WEAR_NONE`, so an unbounded `s = 255` would answer *true* for
/// every non-armor item — the hardening the merge-gate judge asked for on
/// `wearable_in`, taken at both copies rather than one.
pub fn wearable_here(catalog: &ItemCatalog, item: u16, s: usize) -> bool {
    s < WEAR_SLOTS && catalog.wear_slot(item as usize) as usize == s + 1
}

/// The container panel's title. `CONT_SELF` has no panel, so it is named
/// for what it means rather than drawn.
pub fn container_title(kind: u8) -> &'static str {
    match kind {
        CONT_BOX => "BOX",
        CONT_SELF => "-",
        // Named rather than left to the fallback, and that is the whole
        // point of touching this function: the wildcard below reads every
        // unknown kind as a bag, so world containers v0 would have
        // titled the haven pad's crate "BAG" with nothing failing. A
        // fallback that is a real answer for one kind is a fallback that
        // lies about the next one.
        CONT_WORLD => "CRATE",
        // Named for the same reason, one kind later. The fallback would
        // have titled a player's own armor "BAG".
        CONT_WEAR => "WORN",
        _ => "BAG",
    }
}

/// What wear-container slot `s` is called, for the paperdoll's caption.
///
/// **Keyed on the sim's own constants, not on the index.** `Player::worn`
/// is zero-based and `ArmorDef::slot` is one-based, which is the one thing
/// about this container that is easy to get backwards — `combat::worn_pct`
/// and `combat::wearable_in` both spell the relation `slot == s + 1`, and
/// so does this, so a third slot lands here as a compile-visible gap
/// rather than as a caption that is off by one.
///
/// A slot past `WEAR_SLOTS` has no name; it cannot be drawn (the panel
/// walks `WEAR_SLOTS`) and it cannot be addressed (`slots_in`), so the
/// fallback is a placeholder rather than a guess at what a fourth slot
/// would be called.
pub fn wear_slot_label(s: usize) -> &'static str {
    match u8::try_from(s).map(|s| s + 1) {
        Ok(WEAR_HEAD) => "HEAD",
        Ok(WEAR_BODY) => "BODY",
        _ => "-",
    }
}

/// Width of the container panel's grid, in cells. A box is narrower than
/// the view that carries it: the wire ships `INV_SLOTS` slots whatever kind
/// is open, and the tail stays zero for a box — so a panel that drew all 30
/// would draw twelve slots and eighteen lies.
///
/// **Derived from `slots_in` rather than listed**, which is the same move
/// the re-export at the top of this file makes and for the same reason.
/// The old body named `CONT_BOX` and answered `GRID_COLS` for everything
/// else; that is a hand-kept mirror of `sim_core`'s width table, and a
/// two-slot wear container would have drawn as six — four of them lies,
/// which is the defect the paragraph above describes arriving at the next
/// kind. This form is identical for all four older kinds (30 and 12 both
/// clamp to `GRID_COLS`; 12 was already reaching `BOX_SLOTS.min`) and
/// correct for the fifth without naming it.
pub fn container_cols(kind: u8) -> usize {
    slots_in(kind).min(GRID_COLS)
}

/// The deployable item standing at a box handle, if the client is already
/// drawing one.
///
/// **No new wire.** `CONT_BOX`'s handle is `box_key(cx, cz, level)` — an
/// address, not an id, and deliberately so (`sim_core::inventory`) — and the
/// deploy sync the client draws every box from carries the same three
/// numbers plus the baked row. So the panel can name the box it opened out
/// of what is already on screen.
///
/// **The arch check is what makes the address unambiguous**, not tidiness: a
/// handle names a cell and a level, and `DeployRec` also carries a `loc`, so
/// a hearth and a box sharing one cell share one key. Filtering to `ARCH_BOX`
/// picks the one the handle can actually mean.
///
/// `defs_have` is the watermark — the def table drips in over the first
/// seconds of a session, and a row past it is a row of zeroes whose `arch`
/// reads as `ARCH_BAG`. Reading it anyway would name a box after the wrong
/// item, which is worse than not naming it.
pub fn box_item_at(
    handle: u32,
    deploys: &[DeployRec],
    defs: &DeployContent,
    defs_have: u16,
) -> Option<u16> {
    deploys.iter().find_map(|d| {
        if box_key(d.cx, d.cz, d.level) != handle || u16::from(d.row) >= defs_have {
            return None;
        }
        let def = &defs.defs[d.row as usize];
        (def.arch == ARCH_BOX).then_some(def.item)
    })
}

/// What the LOOT panel's name bar says — the container's own name, in the
/// reference frame's caps.
///
/// The reference names the thing you opened (`LARGE WOODEN BOX`) rather
/// than its category, and the name is the only thing on that screen telling
/// two boxes apart. Ours comes from `content/items.toml` by way of the
/// catalog, so a shard that renames its boxes renames this bar with no code
/// change — wall 7.
///
/// Falls back to [`container_title`] whenever the name cannot be *known*:
/// a bag or a crate, whose handles are not addresses this side can resolve;
/// a box whose deploy record or def row has not arrived yet; and an item
/// index the catalog has no name for, which `item_label` renders as `#12`
/// and which would read as a defect in 12 px caps. A generic word is a
/// worse label and an honest one.
pub fn container_name(
    kind: u8,
    handle: u32,
    deploys: &[DeployRec],
    defs: &DeployContent,
    defs_have: u16,
    catalog: &ItemCatalog,
) -> String {
    if kind == CONT_BOX {
        if let Some(item) = box_item_at(handle, deploys, defs, defs_have) {
            let name = crate::ui::craft::item_label(catalog, item);
            if !name.starts_with('#') {
                return name.to_uppercase();
            }
        }
    }
    container_title(kind).to_string()
}

/// The multiplier a filled cell draws, or `None` when there is nothing worth
/// drawing.
///
/// Two rules, both the reference's own: **a count of one is not drawn** — a
/// screen of tools would otherwise read as a screen of numbers — and the
/// count that *is* drawn carries its `x`. The prefix is not decoration: a
/// bare `3` sitting in the corner of a picture of an arrow is ambiguous with
/// a quality, a tier or a slot number, and every reference frame this panel
/// is measured against writes `x3`.
pub fn count_badge(count: u16) -> Option<String> {
    (count > 1).then(|| format!("x{count}"))
}

/// The durability pip's fill fraction, or `None` when no pip is drawn
/// (`NOW.md` §0dur item 1) — [`count_badge`]'s shape for the other number a
/// cell carries.
///
/// The reference's rule, and all three states are it: a thin bar under the
/// icon, **visible only when the item is worn**. An item that carries no
/// condition (`cond_max == 0` — wood, stone, every stackable) never draws
/// one; a pristine tool (`cond >= cond_max`, and `>` only off a smuggled
/// save) draws none either, because a full bar under every fresh tool is a
/// screen of bars; a worn tool draws `cond / cond_max`, and a dead one
/// (`cond == 0` under a nonzero ceiling) draws the bar EMPTY — the warning
/// `REFUSE_G_BROKEN` otherwise gives only at the moment the swing bounces.
///
/// **The ceiling arrives now** (wire v46, 2026-08-17): the catalog drip
/// carries `cond_max` beside every name, so a panel's call is
/// `pip_fraction(stack.cond, core.catalog.cond_max(stack.item as usize))`.
/// Until that bump this function had no possible caller — the client held
/// `cond` per stack (wire v42) and NO ceiling to divide by, because the
/// catalog was names-only and the client links no content crate. A
/// session-learned ceiling (max `cond` seen per item) was considered and
/// REFUSED on the way: a tool looted half-worn would read pristine, which
/// is the exact lie the pip exists to prevent; and hardcoding the per-item
/// table here is wall 7's business.
///
/// **Drawn since 2026-08-17** by all three cells that hold a stack — the
/// hotbar (`render::hud`), the inventory and container grids and the drag
/// ghost (`render::panels::inv`) — and by nothing else, which is a property
/// of call sites rather than of this value, so `tests/ui.rs` §Q scans for
/// them. The colours and the bar's height are
/// `render::panels::{PIP_FILL, PIP_TROUGH, PIP_H_PX}`; this function decides
/// only whether there is a bar and how full it is.
pub fn pip_fraction(cond: u16, cond_max: u16) -> Option<f32> {
    (cond < cond_max).then(|| cond as f32 / cond_max as f32)
}

/// Where the thing in your hand is drawn, given the cursor and the tile's
/// edge: **centred on the pointer**, which is what every game this client's
/// players arrive from does.
///
/// It used to hang off to the lower right, and that is the bug this function
/// exists to close rather than a taste call — an offset ghost is a ghost the
/// player aims with by parallax, and at the screen's right edge it walks off
/// the window while the cursor is still inside it. Centred, the tile *is*
/// the cursor's payload: the cell under the pointer is the cell the drop
/// addresses, and the picture over it is what will land there.
///
/// The tile never eats the pointer (`Pickable::IGNORE` at every node of it),
/// so covering the target is free — and the OS cursor draws above the
/// window, so the arrow stays visible on top of the item exactly as it does
/// in the reference frame.
pub fn ghost_origin(cursor_x: f32, cursor_y: f32, size: f32) -> (f32, f32) {
    (cursor_x - size * 0.5, cursor_y - size * 0.5)
}
