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
use sim_core::inventory::{
    deposit_refused, is_own, CONT_BOX, CONT_MAX, CONT_SELF, CONT_WEAR, CONT_WORLD,
};
use sim_core::limits::{HOTBAR_SLOTS, INV_SLOTS, WEAR_SLOTS};

/// Slots addressable in a container of `kind` — `sim_core::inventory`'s
/// `slots_in`, re-exported rather than mirrored so the two cannot drift.
pub use sim_core::inventory::slots_in;
/// May a player put something into a container of this kind — the sim's
/// own predicate, re-exported for [`slots_in`]'s reason. The panel draws
/// the answer *before* the release (a red edge on a crate's cells), and a
/// second list of which containers are loot-only is the drift this
/// re-export exists to make impossible.
pub use sim_core::inventory::takes_deposits;

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
    /// **As much as the destination was measured to take** — minted only
    /// by [`quick_move`], never by a drag, which is why it carries a
    /// number where the other three carry a rule.
    ///
    /// The number comes off the catalog's `stack_max` and the destination
    /// slot's own count, so it is *the room that is there* rather than a
    /// hope: `plan_move` refuses a merge of `count > room` outright
    /// instead of clamping (a clamp is the silent divergence the whole
    /// move verb exists to avoid), so the panel has to do the measuring
    /// before it sends. Still bounded by `held` below, like the other
    /// three — a count no stack can back is refused by [`move_args`]
    /// whatever asked for it.
    Fit(u16),
}

impl Grab {
    /// Units this gesture takes out of a stack of `held`. Zero in, zero out
    /// — an empty slot is refused by [`move_args`], not clamped here.
    pub fn units(self, held: u16) -> u16 {
        match self {
            Grab::All => held,
            Grab::Half => held.div_ceil(2),
            Grab::One => held.min(1),
            Grab::Fit(n) => n.min(held),
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
/// 6. **A deposit into a container that takes none** (`takes_deposits`,
///    wire v64), on both of a move's landing sites — a swap out of a
///    loot-only crate puts the occupant *into* it. The sim's predicate,
///    not a second list; the panel says the crate's own sentence before
///    a release ever reaches this ladder.
/// 7. **A count of zero, or more than the source holds.** The sim does not
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
    worn: &[ItemStack],
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
    // 6 · the container that takes nothing a player hands it
    //     (`REFUSE_M_NO_INPUT`, wire v64). Asked with the sim's own
    //     predicate and on both of a move's landing sites, because a
    //     swap out of a crate deposits the occupant into it.
    //
    //     **The panel prints the sentence before it reaches here.**
    //     `refusal_text(REFUSE_M_NO_INPUT)` is what a release over a
    //     crate says, so the player hears the crate's own words and not
    //     this ladder's generic "cannot be addressed". This step is the
    //     backstop for that, and for every other caller: without it a
    //     quick-move aimed at a crate would cross the wire to be refused,
    //     which is the round trip this whole file exists to save.
    let src_view: &[ItemStack] = match from_kind {
        CONT_SELF => inv,
        CONT_WEAR => worn,
        _ => cont,
    };
    let dst_view: &[ItemStack] = match to_kind {
        CONT_SELF => inv,
        CONT_WEAR => worn,
        _ => cont,
    };
    if deposit_refused(
        from_kind,
        to_kind,
        src_view.get(from_slot).copied().unwrap_or_default(),
        dst_view.get(to_slot).copied().unwrap_or_default(),
    ) {
        return None;
    }
    // 7 · the count, read from the source container's own view.
    //
    //     Three views since the body moved off the ground subscription
    //     (`NOW.md` §0eq item 4): the pack, the body, and whatever is
    //     open. This was a two-way pick on `== CONT_SELF` while the wear
    //     slots lived in `cont`, and leaving it that way would have read
    //     a helmet's count out of the open box's slot 0 — the right
    //     value in the wrong container, which is the positional-payload
    //     shape and would have shipped a plausible `count`.
    //
    //     `get` rather than an index: step 3 already bounded `from_slot`
    //     against `slots_in(from_kind)`, but `worn` is a slice and its
    //     width is no longer proved by its type.
    //
    //     `src_view` is the same pick step 6 already made — one `match`
    //     rather than two. Two copies of "which array holds this kind"
    //     is the shape that put a helmet's count where a box's belonged.
    let held = src_view.get(from_slot).map_or(0, |s| s.count);
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
        REFUSE_M_COUNT, REFUSE_M_EMPTY, REFUSE_M_NO_CONTAINER, REFUSE_M_NO_INPUT, REFUSE_M_NO_ROOM,
        REFUSE_M_OVEN, REFUSE_M_REACH, REFUSE_M_SLOT, REFUSE_M_UNSTACKABLE, REFUSE_M_WEAR,
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
        // The container's own refusal (v64). Named after the crate
        // because `CONT_WORLD` is the only kind that gives it today and
        // `container_title` calls that panel `CRATE` — the word on the
        // screen and the word in the line are the same one.
        REFUSE_M_NO_INPUT => "that crate gives loot and takes none",
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
/// `worn` is the client's view of the `CONT_WEAR` container. It is a
/// **slice** rather than a fixed array because the view moved: it was
/// `cont`, `INV_SLOTS` wide with a tail of zeros, until the body got its
/// own `WEAR_SLOTS`-wide stream (2026-08-28, `NOW.md` §0eq item 4). The
/// `take(WEAR_SLOTS)` is unchanged and is why both shapes are correct
/// here — it reads the sim's own width and nothing past it, so a
/// smuggled stack in slot 7 protects nobody either way.
pub fn worn_pct(catalog: &ItemCatalog, worn: &[ItemStack]) -> u32 {
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

/// Is a ground container open — is the player **looting**?
///
/// One name for a state two parts of the screen key off: the container
/// grid is drawn, and the crafting half is not (`screen_title`). `is_own`
/// rather than `!= CONT_SELF`, which is `sim_core`'s own distinction and
/// the reason it exists — the body is a container the player carries, and
/// it is never what `ClientCore::cont_kind` holds, so the two spellings
/// agree today and only one of them keeps agreeing.
pub fn looting(cont_kind: u8) -> bool {
    !is_own(cont_kind)
}

/// The screen title over the inventory panel.
///
/// **`CRAFTING` only when the crafting half is actually drawn.** The title
/// names the region under it (this file's own rule, and why it stopped
/// saying `INVENTORY` — the grids have their own heads), so with a
/// container open and the recipe browser gone, `CRAFTING` would label a
/// row of grids and `INVENTORY` would be the screen's third `INVENTORY`.
///
/// The operator, 2026-09-16, looking at the crafting panel over a bag's
/// slots: *"we shouldnt show crafting"*. So the word names the verb the
/// screen is for instead, which is the one thing on it that is not also a
/// heading somewhere else.
pub fn screen_title(cont_kind: u8) -> &'static str {
    if looting(cont_kind) {
        "LOOTING"
    } else {
        "CRAFTING"
    }
}

/// What a right-click on a slot with no drag resolves to.
///
/// Three outcomes and the gesture has all three, which is why this is one
/// value rather than an `if` inside a Bevy system: the whole decision is
/// testable from the code tier (`tests/ui.rs` §T) and the system does
/// nothing but send what it is handed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quick {
    /// Send this move. The destination slot is the panel's own pick — see
    /// [`quick_move`] for how it is measured.
    Send(MoveArgs),
    /// No container is open, so the gesture keeps its old meaning: use
    /// the item in this **inventory** slot (`ACT_CONSUME`).
    Use(usize),
    /// Nothing to send, and this is the line to print. Never silence: a
    /// panel that cannot say why it did nothing is the dark-panel defect,
    /// and a right-click that looks like it failed is exactly where a
    /// player assumes the game is broken.
    Refused(&'static str),
}

/// Where a right-click sends a stack, with a container open.
///
/// **The gesture the operator asked for** (2026-09-16: *"right clicking
/// should put it into ur inventory u dont have to drag"*), and it is the
/// reference's too — a right-click on a slot in an open loot panel
/// transfers the stack rather than using it.
///
/// The rule is one sentence: **with a ground container open, a right-click
/// moves a stack to the other side.** Out of the container into the pack,
/// out of the pack (or off the body) into the container. With nothing
/// open it is [`Quick::Use`], which is the gesture this panel already had.
///
/// ## Picking the slot is the whole of the work
///
/// `plan_move` refuses a merge it cannot complete (`count > room` is
/// `REFUSE_M_NO_ROOM`, never a clamp), so a destination is only a
/// destination if the arithmetic is done first:
///
/// 1. **The first slot holding the same item with room**, and the count is
///    that room — so right-clicking 40 wood into a pack holding 990 of a
///    1,000 stack moves ten and leaves thirty, rather than being refused.
///    Same-item first, because the alternative scatters a resource across
///    fresh slots beside the pile it belongs in, which is the thing that
///    makes a quick-move worse than a drag.
/// 2. **Otherwise the first empty slot**, and the count is the whole
///    stack. An empty slot always takes it: the source stack was built
///    under the sim's own ceiling.
/// 3. **Otherwise nothing**, with a line saying so.
///
/// ⚠ **A `stack_max` of 0 means "I do not know", not "unstackable"** —
/// an undelivered catalog row reads as zero, and the two are
/// indistinguishable here by construction (`ItemRow::stack_max`). Such a
/// ceiling measures no room anywhere, so step 1 finds nothing and the
/// stack lands in an empty slot, where the count needs no ceiling to be
/// safe and the sim answers `REFUSE_M_UNSTACKABLE` if the item genuinely
/// has no ladder. Reading 0 as unstackable *here* would instead refuse
/// every quick-move for the first seconds of a session while the catalog
/// drips in — the shape that is hardest to reproduce and easiest to
/// ship, which is why there is a gate on it
/// (`an_undripped_row_lands_in_an_empty_slot_rather_than_refusing`) even
/// though no current branch can get it wrong.
///
/// One move, one slot, because the wire's move verb addresses one slot.
/// A stack that half fits leaves a remainder in place and a second
/// right-click moves it on — which is honest, and is what the sim would
/// have to be asked twice for anyway.
#[allow(clippy::too_many_arguments)]
pub fn quick_move(
    cont_kind: u8,
    cont_handle: u32,
    from_kind: u8,
    from_slot: usize,
    catalog: &ItemCatalog,
    inv: &[ItemStack; INV_SLOTS],
    cont: &[ItemStack; INV_SLOTS],
    worn: &[ItemStack],
) -> Quick {
    let view = |kind: u8| -> &[ItemStack] {
        match kind {
            CONT_SELF => inv,
            CONT_WEAR => worn,
            _ => cont,
        }
    };
    let src = view(from_kind).get(from_slot).copied().unwrap_or_default();

    // Nothing open: the old gesture, and only out of the pack. A
    // right-click on a worn piece with no container open stays inert
    // rather than becoming an unequip nobody asked for.
    if !looting(cont_kind) {
        return if from_kind == CONT_SELF {
            Quick::Use(from_slot)
        } else {
            Quick::Refused("nothing is open to move that into")
        };
    }
    if src.count == 0 {
        return Quick::Refused("there is nothing in that slot");
    }

    // The other side. A move addresses one ground container
    // (`move_args` step 2), so the pack is where anything in the
    // container goes and the container is where anything on the player
    // goes — including off the body, which is the reference's rule too.
    let to_kind = if is_own(from_kind) {
        cont_kind
    } else {
        CONT_SELF
    };
    if !takes_deposits(to_kind) {
        return Quick::Refused(refusal_text(sim_core::inventory::REFUSE_M_NO_INPUT as u8));
    }

    let dst = view(to_kind);
    let cap = catalog.stack_max(src.item as usize);
    let width = slots_in(to_kind);
    let mut empty = None;
    for slot in 0..width {
        let there = dst.get(slot).copied().unwrap_or_default();
        if there.count == 0 {
            if empty.is_none() {
                empty = Some(slot);
            }
            continue;
        }
        // A merge. **An unknown ceiling refuses one by construction**
        // rather than by a guard: `cap` is 0 for a row that has not
        // dripped in, `there.count` is at least 1 in this branch, and
        // `saturating_sub` floors at zero — so an `if cap > 0` beside
        // this would read as load-bearing and change nothing. Same
        // sentence `plan_move` writes about the same call, one crate
        // over.
        if there.item == src.item {
            let room = cap.saturating_sub(there.count);
            if room > 0 {
                return finish(
                    cont_handle,
                    from_kind,
                    from_slot,
                    to_kind,
                    slot,
                    Grab::Fit(room),
                    inv,
                    cont,
                    worn,
                );
            }
        }
    }
    match empty {
        Some(slot) => finish(
            cont_handle,
            from_kind,
            from_slot,
            to_kind,
            slot,
            Grab::All,
            inv,
            cont,
            worn,
        ),
        None => Quick::Refused("no room for that on the other side"),
    }
}

/// Marshal a picked destination through [`move_args`], so a quick-move is
/// held to every refusal a drag is — in the same order, by the same
/// constructor. **The panel has no second road to `MoveArgs`**, which is
/// what keeps the ordering trap closed for a gesture that picks its own
/// target.
#[allow(clippy::too_many_arguments)]
fn finish(
    bag: u32,
    from_kind: u8,
    from_slot: usize,
    to_kind: u8,
    to_slot: usize,
    grab: Grab,
    inv: &[ItemStack; INV_SLOTS],
    cont: &[ItemStack; INV_SLOTS],
    worn: &[ItemStack],
) -> Quick {
    match move_args(
        bag, from_kind, from_slot, to_kind, to_slot, grab, inv, cont, worn,
    ) {
        Some(args) => Quick::Send(args),
        // Unreachable through the walk above, and reported rather than
        // swallowed if it ever is: a slot this side picked and this side
        // then refused is a bug in the picking, and a silent one would
        // read to a player as a right-click that does nothing.
        None => Quick::Refused("that move cannot be addressed from here"),
    }
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

/// The name bar's text, or `None` when it would only repeat the head.
///
/// **Two of the three ground kinds have no instance name to give.** A
/// bag's handle is an id and a crate's is a cell key, so neither resolves
/// through the deploy set and [`container_name`] answers with the generic
/// title for both (`tests/ui.rs` §P states that, deliberately) — and the
/// panel drew that title twice, once as the section head and once in the
/// bar under it. `BAG` over `BAG`, in the operator's own 2026-09-16
/// frame, which is the defect `panels/inv.rs`'s own doc names about the
/// word `INVENTORY` arriving one panel later.
///
/// A box gets the same treatment when its def row has not dripped in yet:
/// `container_name` falls back to `BOX`, and a bar repeating it says
/// nothing while looking like it should.
pub fn container_bar(kind: u8, name: &str) -> Option<&str> {
    (name != container_title(kind)).then_some(name)
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
/// would be called. `s` is bounded here for the same reason
/// `wearable_here` bounds it and not one step later: `s + 1` on a `u8`
/// overflow-panics in debug and wraps to 0 in release, and `0` is
/// `WEAR_NONE` — so an unbounded `s = 255` is a panic in one build and a
/// silent `"-"` in the other, which is two behaviours for one input. The
/// third copy of the `s + 1` shape, hardened like the other two.
pub fn wear_slot_label(s: usize) -> &'static str {
    if s >= WEAR_SLOTS {
        return "-";
    }
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
