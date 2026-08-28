//! The reliable event lane, S→C (NETCODE.md §2: the bidi stream carries
//! "chunk event fan-out · chat · transactions" — this module is that
//! lane's v1 schema, opened by the gather slice). One datagram-style kind
//! (`KIND_EVENT`) with a subtype field: gather payouts, own-inventory
//! updates, scatter-slot harvested/respawned facts, the batched
//! harvested-set join sync, and the item-name catalog.
//!
//! Everything here rides length-prefixed frames on the handshake bidi
//! stream (ordered + reliable), so there is no ack/baseline machinery:
//! a message arrives exactly once, in order, or the connection is dead.
//! Encode and decode are total and allocation-free like the datagrams —
//! the client decodes server bytes, the server encodes on the sim thread.

use crate::bits::{BitReader, BitWriter, WireError};
use crate::chat::{read_text, write_text, ChatText};
use crate::loc_max;
use crate::{
    expect_zero_padding, BUILD_CELL_BITS, BUILD_LEVEL_BITS, BUILD_LOC_BITS, DEPLOY_ROW_BITS,
    DMG_BAND_BITS, KIND_BITS, KIND_EVENT, PIECE_ROW_BITS, PLATE_BIAS, PLATE_BITS, POS_XZ_BITS,
    POS_Y_BIAS, POS_Y_BITS,
};
use sim_core::backpack::BackpackRec;
use sim_core::build::{
    BuildContent, PieceDef, PieceRec, DMG_BANDS, LOC_EDGE_ZLO, MAT_METAL, SHAPE_TRI_ROOF,
};
use sim_core::craft::{CraftContent, CraftJob, RecipeDef, STATION_MAX};
use sim_core::deploy::{
    BagAnchor, DeployContent, DeployDef, DeployRec, ARCH_WORKBENCH3, BAG_CAP, PLACE_DOOR,
};
use sim_core::gather::ItemStack;
use sim_core::inventory::{slots_in, CONT_MAX, CONT_SELF};
use sim_core::limits::{
    CRAFT_QUEUE, HEARTH_STOCK_ROWS, INV_SLOTS, MAX_BUILD_COORD, MAX_BUILD_LEVELS, MAX_DEPLOY_COSTS,
    MAX_DEPLOY_DEFS, MAX_ITEM_DEFS, MAX_PIECE_COSTS, MAX_PIECE_DEFS, MAX_RECIPES,
    MAX_RECIPE_INPUTS, MAX_RESEARCH_ROWS,
};
use sim_core::research::{ResearchRow, NO_RECIPE};

/// Longest event-lane message. Sized by the worst subtype (a full catalog
/// batch ≈ 280 B since v46's per-row `cond_max`, a full slot-sync batch
/// ≈ 258 B) with headroom; the client-side framer refuses past it.
/// Registered in DECISIONS.md §open.
pub const MAX_EVENT_MSG_BYTES: usize = 320;

/// Harvested cells one sync message carries (join sync is drip-fed, one
/// message per client per tick — server core). Overflow policy: the next
/// message continues the walk.
pub const SLOT_SYNC_BATCH: usize = 64;

/// Item names one catalog message carries.
pub const CATALOG_BATCH: usize = 8;

/// Recipe rows one recipes message carries (a full row is ~22 B; four
/// keep the drip well under the message cap).
pub const RECIPE_BATCH: usize = 4;

/// Research rows one research-rows message carries (tech tree v0). A row
/// is 6 B flat, so four ride far under the cap; the batch matches
/// `RECIPE_BATCH` because the two tables drip side by side on join.
pub const RESEARCH_BATCH: usize = 4;

/// Placed-piece records one sync message carries (a record is 33 bits;
/// 32 keep the batch ≈ 134 B, well under the message cap). The join walk
/// is drip-fed like the harvested-set sync.
pub const PIECE_SYNC_BATCH: usize = 32;

/// Piece-def rows one defs message carries (a full row is ~11 B).
pub const PIECE_DEFS_BATCH: usize = 6;

/// Placed-deployable records one sync message carries (a record is 31
/// bits — address, row, open, locked; 24 keep the batch ≈ 94 B, well
/// under the message cap). The join walk is drip-fed like the piece sync.
pub const DEPLOY_SYNC_BATCH: usize = 24;

/// Deploy-def rows one defs message carries (a full row is ~5 B).
pub const DEPLOY_DEFS_BATCH: usize = 8;

/// Death-backpack records one sync message carries (a record is 80 bits
/// — id + the three position quanta; 16 keep the batch ≈ 162 B, well
/// under the message cap). The join walk is drip-fed like the deploy
/// sync, and for the same reason: a bag standing when you arrive must be
/// there when you look, without a burst at the door.
pub const BAG_SYNC_BATCH: usize = 16;

/// Longest item display name on the wire; the catalog bake refuses past
/// it (content names are short by construction — CONTENT.md §2).
pub const MAX_ITEM_NAME_BYTES: usize = 24;

/// Slots one container-sync message carries. Not a drip constant like the
/// syncs above it, and that is the point: the widest container is
/// `INV_SLOTS` and a slot is 53 bits (v42 added 16 of condition), so a
/// *whole* container is ≈ 205 B —
/// comfortably inside `MAX_EVENT_MSG_BYTES` — and a cursor would buy a
/// second walk to restart, a second reset flag to get wrong, and a window
/// in which a panel is drawn half full. One message, one truth.
pub const CONT_SYNC_BATCH: usize = INV_SLOTS;

/// Widened 5 → 6 with `PROTO_VER` 13 (piece damage v0): the struct-hit
/// subtype was the 31st of the 32 a 5-bit field holds, which left one code
/// and no room for the unknown-subtype probe to have anything to probe.
/// One bit on every event message, taken while the goldens were being
/// regenerated anyway — the cheapest moment there will ever be.
const SUB_BITS: u32 = 6;
const SUB_GATHER: u32 = 0;
const SUB_INV: u32 = 1;
const SUB_SLOT_HARVESTED: u32 = 2;
const SUB_SLOT_RESPAWNED: u32 = 3;
const SUB_SLOT_SYNC: u32 = 4;
const SUB_CATALOG: u32 = 5;
const SUB_WEAK_MARK: u32 = 6;
const SUB_CRAFT_Q: u32 = 7;
const SUB_CRAFT_DONE: u32 = 8;
const SUB_CRAFT_REFUSED: u32 = 9;
const SUB_RECIPES: u32 = 10;
const SUB_PIECE_PLACED: u32 = 11;
const SUB_PIECE_SYNC: u32 = 12;
const SUB_BUILD_REFUSED: u32 = 13;
const SUB_PIECE_DEFS: u32 = 14;
const SUB_DEPLOY_PLACED: u32 = 15;
const SUB_DEPLOY_SYNC: u32 = 16;
const SUB_DEPLOY_REFUSED: u32 = 17;
const SUB_DEPLOY_DEFS: u32 = 18;
const SUB_PIECE_REMOVED: u32 = 19;
const SUB_DEPLOY_REMOVED: u32 = 20;
const SUB_STOCK: u32 = 21;
const SUB_DOOR: u32 = 22;
const SUB_CHAT: u32 = 23;
const SUB_HIT: u32 = 24;
const SUB_HEALTH: u32 = 25;
const SUB_DEATH: u32 = 26;
const SUB_BAG_DROPPED: u32 = 27;
const SUB_BAG_SYNC: u32 = 28;
const SUB_BAG_REMOVED: u32 = 29;
const SUB_STRUCT_HIT: u32 = 30;
/// The survival clock's three (wire v14, survival.rs). `SUB_VITALS` is the
/// meter pair; the other two are the eat verb's acknowledgement and its
/// refusal, which follow craft/build/deploy's posture that a verb which
/// did nothing says so rather than swallowing the press.
const SUB_VITALS: u32 = 31;
const SUB_CONSUMED: u32 = 32;
const SUB_CONSUME_REFUSED: u32 = 33;
/// The drink verb's acknowledgement (wire v15, survival.rs). Its refusal
/// rides `SUB_CONSUME_REFUSED` — one refusal channel for the whole
/// survival module, because the HUD's answer to every one of them is the
/// same line — but the acknowledgement is its own subtype: a drink from a
/// salt sea *costs* hp, and `SUB_HEALTH` is absolute, so a client that
/// only heard the new hp could not name what took it.
const SUB_DRANK: u32 = 34;
/// The death screen closed and a body woke (wire v16, world.rs). 36th of
/// the 64 a 6-bit field holds, so no other event message moved for it.
const SUB_RESPAWN: u32 = 35;
/// The move acknowledgement and its refusal (wire v17, `inventory.rs`).
/// 37th and 38th of the sixty-four v13's width holds, so nothing moved.
const SUB_MOVED: u32 = 36;
const SUB_MOVE_REFUSED: u32 = 37;
/// An open container's contents (wire v19). 39th of the sixty-four, so
/// nothing moved for it.
///
/// **The first S→C message addressed to one client on purpose rather than
/// by accident.** Every other unicast on this lane is an own-fact — your
/// gather, your health, your refusal — and is unicast because nobody else
/// would want it. This one is unicast because everybody else *would*: a
/// box's contents fanned out to AOI is a raider reading a base's stock
/// from outside its walls, which is ESP with a nicer name, plus a
/// bandwidth bill for a panel nobody has open. The audience is exactly
/// the client that asked, for exactly as long as the server can still
/// prove it is in reach.
const SUB_CONT_SYNC: u32 = 38;
/// A piece was bought back to full (`EV_PIECE_REPAIRED`, wire v20).
/// Broadcast, and `SUB_STRUCT_HIT`'s mirror image field for field, so the
/// two writers of a client's hp mirror read the same.
const SUB_PIECE_REPAIRED: u32 = 39;
/// A charge was planted and its fuse is burning (`EV_CHARGE_PLACED`, wire
/// v23). Broadcast, and that is the whole point of it: a burning fuse is
/// the one fact in this game the *defender* needs more urgently than the
/// actor. `SUB_PIECE_REPAIRED`'s address layout, with the fuse where its
/// `healed`/`hp` pair sits.
const SUB_CHARGE_PLACED: u32 = 40;
/// The oven at the address is now lit or out (oven v0, `sim-core/oven.rs`).
/// `SUB_DOOR`'s shape minus the `loc` — an oven stands on the plane, never
/// on an edge — plus the actor, because "who lit this" is the one thing a
/// door's event does not have to carry and a fire's does: zero means the
/// fire ran out of fuel and snuffed itself, and a client that could not
/// tell that from a hand on the switch would owe a toast it should not
/// print.
const SUB_OVEN: u32 = 41;
/// Somebody knocked (lock v1, wire v30). Broadcast, `SUB_DOOR`'s address
/// with a player id where its three state bits sit.
const SUB_KNOCK: u32 = 42;
/// A correct code (lock v1, wire v30). **Own-fact**, and the reason it is
/// its own subtype rather than a fourth bit on `SUB_DOOR`: `SUB_DOOR` is a
/// broadcast, and a grant is true of exactly one recipient.
const SUB_AUTH: u32 = 43;
/// The highest live subtype, named rather than counted — `world.rs`'s
/// `EV_MAX` discipline applied to the wire half.
///
/// `trailing_garbage_and_unknown_subtype_are_malformed` probes "the first
/// unused subtype", and it had that written as `SUB_RESPAWN + 1`. Two
/// subtypes landed above `SUB_RESPAWN` and the probe silently became a
/// probe of a **live** code — it caught it here only because the new
/// decoder arm rejected its all-zero payload, which is luck, not a gate.
/// Deriving the probe from this constant is what makes it stay a probe.
/// A blueprint learned (research v0). Own-fact, `SUB_AUTH`'s posture:
/// what a rival has unlocked is their tech level and nobody else's
/// business.
const SUB_RESEARCH: u32 = 44;
/// A research request bounced, with its `research::REFUSE_R_*` reason.
const SUB_RESEARCH_REFUSED: u32 = 45;
/// The whole known-blueprint mask, restated. Sent on join and on any
/// change rather than only as a delta, for the reason `structures.rs`
/// reads the mirror rather than the deltas: a client that missed one
/// `SUB_RESEARCH` would grey a recipe the player has paid for, forever,
/// with no event left to correct it. Sixty-four bits is cheaper than that
/// failure and it makes the state unloseable by construction.
const SUB_KNOWN: u32 = 46;
/// An arrow left a bow (`sim-core/ranged.rs`, wire v33). Broadcast — an
/// arrow in the air is a world fact like a door swinging.
///
/// The five fields are exactly what redraws the sim's own arc and nothing
/// more: the shooter (whose snapshot position, plus the constant eye
/// height both sides know, is the origin), the two aim angles, and the
/// round's speed and drop. `client-core` holds no content tables, so the
/// ballistics have to cross; carrying them also means the tracer
/// integrates the same integers the sim did rather than approximating
/// them. See `world.rs`'s `EV_SHOT` for the full argument.
const SUB_SHOT: u32 = 47;
/// Research rows `first..first+count`, dripped like the recipe table
/// (wire v38, tech tree v0) — the tree panel's data: what each node
/// costs, which recipe it unlocks, and the `requires` edge the graph is
/// drawn from. The coin item rides every batch header rather than its
/// own message: three spare bytes against a subtype nobody else needs.
const SUB_RESEARCH_ROWS: u32 = 48;
/// A gather swing the node refused (wire v42, `EV_GATHER_REFUSED`).
/// Own-fact, `SUB_CONSUME_REFUSED`'s posture — a button that did nothing
/// says so — and it carries the **held item** beside the reason, because
/// the sentence the HUD owes is *a torch cannot fell a tree*, not "bare
/// hands" (`NOW.md` §0kit item 2). 50th of the 64 a 6-bit field holds.
const SUB_GATHER_REFUSED: u32 = 49;
/// **Your own bags**, whole, with each one's cooldown state (wire v43,
/// bag choice v0). Own-fact and `SUB_KNOWN`'s posture in both halves.
///
/// *Own*, because `DeployRec::owner` is deliberately not on the wire, so
/// the deploy mirror a client already holds says where every bed on the
/// island is and cannot say which are its own. The death screen has to
/// know — a screen that offers "wake on your bag" to a player who has
/// never placed one is a button that always lands on a beach — and the
/// alternative to this message is an owner id on every deployable anyone
/// can see, which hands a raider the census.
///
/// *Whole*, because a delta of a set this small buys nothing and can be
/// lost: a client that missed one placement would offer a bag it does not
/// have, or hide one it does, with no event left to correct it. Eight
/// entries is `BAG_CAP`, which placement already enforces.
///
/// The `ready` bit is a **snapshot at send**, not a subscription: a
/// cooldown lapses on a clock that emits nothing. `world.rs`'s respawn is
/// still the authority and still falls back to the beach, and the client
/// says which anchor answered (`ui::death::woke`), so a stale bit costs a
/// sentence rather than a wrong place to wake up.
/// **50, not 49** — see `PROTO_VER`'s v43 note. This landed on a branch
/// as 49 with `PROTO_VER` 42, and the gather refusal above landed on the
/// trunk as the same two numbers. Two layouts under one version is
/// `worldsave.rs`'s format-3 collision, and it takes the same cure: the
/// trunk's number stands and this takes the next one neither claimed.
const SUB_BAGS: u32 = 50;
/// Where an arrow stopped, and on what kind of surface (wire v45,
/// `sim-core/ranged.rs`). Broadcast, `SUB_SHOT`'s posture: a scuff in the
/// world is a world fact, and unlike the shot it outlives the moment —
/// somebody walking past later is the second audience.
///
/// **The position is absolute and in the entity lane's own quanta**, which
/// is the whole of why this message is four fields and not seven. A mark
/// is placed once and never moves, so there is no baseline to delta
/// against and no interpolation to feed; and reusing `POS_XZ_BITS` /
/// `POS_Y_BITS` / `POS_Y_BIAS` means the window check here is the one
/// `write_bag` already performs, rather than a second opinion about where
/// the island is.
///
/// `surf` is two bits and the fourth value is refused at decode — the
/// posture `SEL_BITS` takes for hotbar 6–7. Three kinds is what the sim's
/// stop test can answer (`ranged::SURF_*`); a fourth would be a new
/// question asked on the hot path, not a spare code waiting here.
const SUB_IMPACT: u32 = 51;
/// A body swung, and every client drawing that body needs to know.
///
/// One `u32` and nothing else — no position, because every body's place is
/// already in the snapshot, and no outcome, because the arm moves whether
/// or not the swing found anything. `SUB_SHOT` is the shape this copies:
/// a broadcast cosmetic fact about someone else's hands.
const SUB_SWING: u32 = 52;
const SUB_MAX: u32 = SUB_SWING;
/// Width of `SUB_IMPACT`'s surface field, and how many values it may say.
///
/// **`SURF_KINDS` is derived from the sim's own last kind rather than
/// typed**, which is the point of it existing: a fourth `SURF_*` added in
/// `ranged.rs` makes the assert below fail to compile here, in the crate
/// that would otherwise have truncated it into a live code. A hand-kept
/// mirror of another crate's surface goes stale — `CLAUDE.md` says so
/// twice, once about a doc comment and once about a grep — so this one is
/// read rather than remembered.
const SURF_BITS: u32 = 2;
const SURF_KINDS: u32 = sim_core::ranged::SURF_BUILT as u32 + 1;
const _: () = assert!(
    SURF_KINDS <= (1 << SURF_BITS),
    "a surface kind past the field width would decode as a different one"
);
/// And the field must hold it. A subtype declared past `SUB_BITS` would
/// truncate on the way out and decode as a *different, live* code — the
/// worst shape of wire drift there is, since both ends would agree on
/// bytes that mean two different things. Compile-time, so it is checked in
/// every build rather than only where a test happens to look.
const _: () = assert!(
    SUB_MAX < (1 << SUB_BITS),
    "an event subtype past the field width would truncate into a live code"
);
/// Container-kind and slot widths on the event lane, deliberately the
/// same two the action lane spends (`lib.rs`: `CONT_KIND_BITS`,
/// `ACTION_SLOT_BITS`). An acknowledgement that could not express an
/// address the client is allowed to *ask* for would be a reconcile hole,
/// so the two lanes carry the address identically or neither is sound.
///
/// **Three since wire v51** (armor v1). The pair was exactly full at v37
/// and `CONT_WEAR` is the kind that had to widen it. Both lanes moved in
/// the one commit for the reason this paragraph already gives: widening
/// only the action lane would let a client ask for an address the
/// acknowledgement could not name back.
const CONT_KIND_BITS: u32 = 3;
const MOVE_SLOT_BITS: u32 = 5;
/// Move-refusal reason width — `inventory::REFUSE_M_*` runs `1..=7` and
/// zero is reserved as "no reason", refused at both ends the way
/// `SUB_CONSUME_REFUSED` already refuses its own zero.
const REFUSE_M_BITS: u32 = 4;
/// Death-cause width (`sim_core::world::DEATH_BY_*`). Widened 2 → 3 at
/// wire v36: the two-bit field had been saturated since v24 (hand, clock,
/// salt, arrow), so the next cause — the mob's bite — was a widening
/// rather than a spare code, exactly as the const block below promised.
/// Three bits hold eight; the unspent values are forgeable and both the
/// encoder and the decoder refuse them — the hotbar selector's posture. A
/// cause is a *closed* set the sim owns: a ninth way to die is the next
/// wire change, which is the point of wall 6.
const DEATH_CAUSE_BITS: u32 = 3;
/// **Derived, never restated.** This was the literal `2`, and a literal
/// here is a copy of a fact that lives in another crate — which is exactly
/// how the 2026-08-05 FAIL shipped: the sim grew `DEATH_BY_ARROW = 3`, the
/// literal stayed 2, and every arrow kill hit the `Err(Range)` arm below.
/// Nothing caught it, because nothing *could*: the golden pins layout and
/// no layout moved. Taking the bound from the sim's own ledger makes the
/// two ends impossible to disagree — a new cause changes this constant on
/// the next build rather than on the next reader's memory.
const DEATH_CAUSE_MAX: u8 = sim_core::world::DEATH_BY_MAX;
/// And the field must hold it — `SUB_MAX`'s compile-time posture, applied
/// to the domain. A cause past `DEATH_CAUSE_BITS` would truncate on the
/// way out and decode as a *different, live* cause: both ends agreeing on
/// bytes that mean two different deaths. Compile-time, so widening the
/// sim's ledger past the field cannot reach a test run — it stops `cargo
/// build`, and the message says which bump was forgotten.
const _: () = assert!(
    (DEATH_CAUSE_MAX as u32) < (1 << DEATH_CAUSE_BITS),
    "sim_core::world::DEATH_BY_MAX no longer fits DEATH_CAUSE_BITS — a new \
     death cause needs the field widened, PROTO_VER bumped and the goldens \
     regenerated in this same commit (CLAUDE.md wall 6)"
);

/// Consume-refusal reason width (`survival::REFUSE_C_*`: three codes
/// today, and zero is reserved as "no reason", which the codec refuses).
const REFUSE_C_BITS: u32 = 4;
/// **Derived, never restated** — `DEATH_CAUSE_MAX`'s posture, for the same
/// reason: a literal here is a copy of a fact that lives in `survival.rs`,
/// and both the encoder and the decoder bound the reason against this, so
/// a copy that drifted would refuse a refusal the sim actually issued.
const REFUSE_C_MAX: u32 = sim_core::survival::REFUSE_C_MAX;
const _: () = assert!(
    REFUSE_C_MAX < (1 << REFUSE_C_BITS),
    "survival::REFUSE_C_MAX no longer fits REFUSE_C_BITS — a new consume \
     refusal needs the field widened, PROTO_VER bumped and the goldens \
     regenerated in this same commit (CLAUDE.md wall 6)"
);
/// Gather-refusal reason width (`gather::REFUSE_G_*`: two codes today,
/// zero reserved as "no reason", refused at both ends — `REFUSE_C_BITS`'s
/// posture exactly).
const REFUSE_G_BITS: u32 = 4;
/// Build-refusal reason width (`build::REFUSE_B_*`: ten codes today, and
/// zero is a live one — `REFUSE_B_PIECE`).
///
/// It was the literal `8` at both ends until v20, which meant the one
/// refusal enumeration with the most members was the one enumeration the
/// domain gate could not see: `every_enumeration_width_is_classified`
/// scrapes `*_BITS` names, and a bare literal has no name to scrape. Same
/// bytes, same width — what changes is that adding an eleventh reason now
/// has a gate to answer to.
const REFUSE_B_BITS: u32 = 8;
const INV_COUNT_BITS: u32 = 5;
const INV_SLOT_BITS: u32 = 5;
const SYNC_COUNT_BITS: u32 = 7;
const CATALOG_TOTAL_BITS: u32 = 7;
const CATALOG_COUNT_BITS: u32 = 4;
const NAME_LEN_BITS: u32 = 5;
const CRAFT_Q_COUNT_BITS: u32 = 3;
/// A `research::REFUSE_R_*` reason: seven values since v38 (the tree
/// verb's parent and bench), and the decoder range-checks rather than
/// trusting the width — the `BAG_GONE_BITS` posture.
const RESEARCH_REFUSE_BITS: u32 = 3;
const RECIPE_TOTAL_BITS: u32 = 7;
const RECIPE_COUNT_BITS: u32 = 3;
/// The research drip's header widths — the recipe drip's, because
/// `MAX_RESEARCH_ROWS == MAX_RECIPES` (`limits.rs` ties them).
const RESEARCH_TOTAL_BITS: u32 = 7;
const RESEARCH_COUNT_BITS: u32 = 3;
/// Craft time crosses as raw ticks (the value the sim runs), not seconds
/// — no cadence coupling; 24 bits cover the bake's ceiling (65535 s ×
/// TICK_HZ ≈ 2 M ticks) with headroom.
const RECIPE_TICKS_BITS: u32 = 24;
/// Widened 2 → 3 at v38: the bench ladder made five stations
/// (`none | workbench1..3 | furnace`) and two bits held four. This is
/// the width that turned `PROTO_VER`, and it moves every recipe row in
/// `SUB_RECIPES` — the goldens moved with it in the same commit.
const STATION_BITS: u32 = 3;
const N_INPUTS_BITS: u32 = 3;
const PIECE_SYNC_COUNT_BITS: u32 = 6;
const PIECE_DEFS_TOTAL_BITS: u32 = 6;
const PIECE_DEFS_COUNT_BITS: u32 = 3;
/// Widened 3 → 4 in wire v40 (triangles v0): catalogue v1 had saturated
/// the 3-bit field — its own domain pin said the triangles could not
/// land without this line. Five of the sixteen values are forgeable now,
/// so both ends range-check against `SHAPE_TRI_ROOF`.
const SHAPE_BITS: u32 = 4;
const MATERIAL_BITS: u32 = 2;
const N_COSTS_BITS: u32 = 2;
/// A deployable's repair-cost row count. Wider than `N_COSTS_BITS` because
/// a deployable is priced from its *recipe*, so the ceiling is
/// `MAX_DEPLOY_COSTS` (= `MAX_RECIPE_INPUTS`, 4) rather than a piece's 2 —
/// and unlike a piece, zero is a live value here: a deployable content
/// quotes no recipe for bakes unpriced and `build::repair` refuses it.
const DEPLOY_COSTS_BITS: u32 = 3;
const DEPLOY_SYNC_COUNT_BITS: u32 = 5;
const DEPLOY_DEFS_TOTAL_BITS: u32 = 5;
const DEPLOY_DEFS_COUNT_BITS: u32 = 4;
/// Widened 3 → 4 in wire v31: `ARCH_RECYCLER` = 8 is the ninth archetype
/// (recycler v0) and three bits held exactly eight. Seven of the sixteen
/// values are now forgeable, so the decoder range-checks the field — the
/// same shape `PLACEMENT_BITS` took one version earlier, and for the same
/// reason: a width with slack is a width that has to be policed.
const ARCH_BITS: u32 = 4;
/// Widened 2 → 3 in wire v28: `PLACE_DOOR` is the fifth placement class
/// (lock v1) and two bits held exactly four. Three of the eight values
/// are now forgeable, so the decoder range-checks the field, which two
/// bits never had to.
const PLACEMENT_BITS: u32 = 3;
const STOCK_COUNT_BITS: u32 = 3;
const BAG_SYNC_COUNT_BITS: u32 = 5;
/// Own-bag count (`SUB_BAGS`). `BAG_CAP` is 8 and a *count* of 8 needs
/// four bits, so 9..15 are forgeable and the decoder refuses them —
/// `CONT_COUNT_BITS`' posture, written down because "8 fits in three
/// bits" is the off-by-one that would truncate a full rack of bags to
/// none and make the death screen quietly stop offering them.
const BAGS_COUNT_BITS: u32 = 4;
const _: () = assert!(
    BAG_CAP < (1 << BAGS_COUNT_BITS),
    "an own-bag count past the field width would truncate a full rack to zero"
);
/// Container-sync slot count. `CONT_SYNC_BATCH` is `INV_SLOTS` = 30, so
/// six bits hold it and the count itself is bounded by the decoder rather
/// than by the width — 31..63 are forgeable and refuse, the way `SUB_INV`
/// refuses a count past `INV_SLOTS`.
const CONT_COUNT_BITS: u32 = 6;
/// Why a bag left: `sim_core::backpack::BAG_GONE_*`, three values today,
/// so the fourth pattern the width holds is forgeable and both ends
/// refuse it against the derived bound below.
const BAG_GONE_BITS: u32 = 2;
/// **Derived, never restated** — `REFUSE_C_MAX`'s posture exactly.
const BAG_GONE_MAX: u32 = sim_core::backpack::BAG_GONE_MAX;
const _: () = assert!(
    BAG_GONE_MAX < (1 << BAG_GONE_BITS),
    "backpack::BAG_GONE_MAX no longer fits BAG_GONE_BITS — a new bag-gone \
     reason needs the field widened, PROTO_VER bumped and the goldens \
     regenerated in this same commit (CLAUDE.md wall 6)"
);

/// One changed inventory slot on the wire.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InvSlot {
    pub slot: u8,
    pub stack: ItemStack,
}

/// The item-index → display-name table (the mapping `content::bake`
/// promises the wire ships). Server bakes it at boot; the client fills
/// its copy from catalog messages. Fixed storage, `MAX_ITEM_DEFS` rows.
///
/// v46 added `cond_max` — the item's condition ceiling in the same u16
/// hundredths `ItemStack::cond` rides the container lanes in — because the
/// client held condition with nothing to divide it by (NOW.md §0dur.1: the
/// catalog dripped names only, no def table carried a ceiling, and the
/// client links no content crate). 0 means the item carries no condition,
/// the same convention `content` and the sim use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ItemCatalog {
    pub names: [[u8; MAX_ITEM_NAME_BYTES]; MAX_ITEM_DEFS],
    pub lens: [u8; MAX_ITEM_DEFS],
    pub cond_max: [u16; MAX_ITEM_DEFS],
    pub count: u16,
}

impl ItemCatalog {
    pub const EMPTY: Self = Self {
        names: [[0; MAX_ITEM_NAME_BYTES]; MAX_ITEM_DEFS],
        lens: [0; MAX_ITEM_DEFS],
        cond_max: [0; MAX_ITEM_DEFS],
        count: 0,
    };

    /// Install one row. Refuses empty, oversize, or out-of-table — the
    /// server's bake path turns this into a refused boot. `cond_max` is a
    /// parameter rather than a second setter so the compiler refuses a row
    /// installed without its ceiling (the omission that kept the column
    /// off the wire for four versions).
    pub fn set(&mut self, idx: usize, name: &[u8], cond_max: u16) -> Result<(), WireError> {
        if idx >= MAX_ITEM_DEFS || name.is_empty() || name.len() > MAX_ITEM_NAME_BYTES {
            return Err(WireError::Range);
        }
        self.names[idx][..name.len()].copy_from_slice(name);
        self.names[idx][name.len()..].fill(0);
        self.lens[idx] = name.len() as u8;
        self.cond_max[idx] = cond_max;
        Ok(())
    }

    pub fn name(&self, idx: usize) -> &[u8] {
        if idx < MAX_ITEM_DEFS {
            &self.names[idx][..self.lens[idx] as usize]
        } else {
            &[]
        }
    }

    /// The condition ceiling for one item index; 0 out of table, matching
    /// the 0-means-no-condition convention in table.
    pub fn cond_max(&self, idx: usize) -> u16 {
        if idx < MAX_ITEM_DEFS {
            self.cond_max[idx]
        } else {
            0
        }
    }
}

impl Default for ItemCatalog {
    fn default() -> Self {
        Self::EMPTY
    }
}

/// One decoded event-lane message. Fixed storage; unused tails stay zero
/// so equality is well-defined (the goldens compare decoded values).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventMsg {
    /// Own gather payout: `added` units of `item` landed (or 0 — full
    /// inventory). The toast, not the truth: `Inv` is authoritative.
    Gather { item: u16, added: u16 },
    /// A gather swing bounced (wire v42): `item` is what was held —
    /// `sim_core::gather::NO_ITEM` means bare hands — and `reason` is a
    /// `sim_core::gather::REFUSE_G_*` code. The item crosses so the HUD
    /// can name the torch instead of saying "bare hands" (`SUB_GATHER_REFUSED`).
    GatherRefused { item: u16, reason: u8 },
    /// Authoritative own-inventory slots that changed since last sent.
    Inv {
        slots: [InvSlot; INV_SLOTS],
        count: u8,
    },
    /// A scatter slot was exhausted — the node vanishes until respawn.
    SlotHarvested { cx: u16, cz: u16 },
    /// A harvested slot's timer arrived — the node stands again.
    SlotRespawned { cx: u16, cz: u16 },
    /// One batch of the harvested-cell walk. `reset` (first batch of a
    /// join or an event-lane resync) clears the client's set first.
    SlotSync {
        reset: bool,
        cells: [(u16, u16); SLOT_SYNC_BATCH],
        count: u8,
    },
    /// Item display names `first..first+count` of a `total`-row table.
    /// Each row carries its condition ceiling too (v46) — the number
    /// `pip_fraction` divides a container lane's `cond` by; 0 means the
    /// item carries no condition.
    Catalog {
        total: u8,
        first: u8,
        count: u8,
        names: [[u8; MAX_ITEM_NAME_BYTES]; CATALOG_BATCH],
        lens: [u8; CATALOG_BATCH],
        cond_max: [u16; CATALOG_BATCH],
    },
    /// Own weak-spot mark after a landed hit (swinger-only): the node's
    /// cell, the next mark heading (u8 over the shared 256-entry yaw LUT,
    /// pointing node → stand point), and whether that hit was a weak hit.
    WeakMark {
        cx: u16,
        cz: u16,
        mark8: u8,
        weak_hit: bool,
    },
    /// Authoritative own craft queue after a change: `count` live jobs
    /// (dense, head first) as (recipe index, units remaining), plus the
    /// head unit's remaining ticks at send time (the client counts down
    /// locally between messages).
    CraftQ {
        jobs: [(u8, u8); CRAFT_QUEUE],
        count: u8,
        eta_ticks: u16,
    },
    /// One craft unit completed: `added` units of `item` landed (0 = full
    /// inventory — the loss is announced). The toast; `Inv` is the truth.
    CraftDone { item: u16, added: u16 },
    /// A blueprint was learned: `recipe` is now craftable by this player,
    /// and `cost` is what it actually burned (research.rs).
    Research { recipe: u16, cost: u16 },
    /// A research request bounced: `reason` is a
    /// `sim_core::research::REFUSE_R_*` code.
    ResearchRefused { reason: u8 },
    /// Every blueprint this player knows, as a bitmask over recipe
    /// indices. The whole mask, not a delta — see `SUB_KNOWN`.
    Known { mask: u64 },
    /// Every bag **this player** has placed, whole, with each one's
    /// cooldown state at send time — the death screen's data. See
    /// `SUB_BAGS` for why it is its own message and not a column on the
    /// deploy record everybody sees.
    Bags {
        bags: [BagAnchor; BAG_CAP],
        count: u8,
    },
    /// A craft request bounced: `reason` is a `sim_core::craft::REFUSE_*`
    /// code (unknown values render as a generic refusal).
    CraftRefused { reason: u8 },
    /// Recipe rows `first..first+count` of a `total`-row table — the
    /// craft menu's data, dripped like the item catalog. Rows decode to
    /// the same `RecipeDef` the sim runs (craft time crosses as ticks).
    Recipes {
        total: u8,
        first: u8,
        count: u8,
        rows: [RecipeDef; RECIPE_BATCH],
    },
    /// Research rows `first..first+count` of a `total`-row table — the
    /// tech tree panel's data, dripped like the recipe table. Rows decode
    /// to the same `ResearchRow` the sim runs, and `coin` is
    /// `ResearchContent::coin`, so the panel can price a node against
    /// the player's own stacks.
    ResearchRows {
        total: u8,
        first: u8,
        count: u8,
        coin: u16,
        rows: [ResearchRow; RESEARCH_BATCH],
    },
    /// A building piece landed (broadcast — pieces are world facts like
    /// slot changes). The record is the sim's own `PieceRec`.
    PiecePlaced { rec: PieceRec },
    /// One batch of the placed-piece walk (join sync / event-lane resync).
    /// `reset` clears the client's piece set first.
    PieceSync {
        reset: bool,
        recs: [PieceRec; PIECE_SYNC_BATCH],
        count: u8,
    },
    /// A place request bounced: `reason` is a `sim_core::build::REFUSE_B_*`
    /// code (unknown values render as a generic refusal).
    BuildRefused { reason: u8 },
    /// Piece-def rows `first..first+count` of a `total`-row table — the
    /// build menu's data, dripped like the recipe table. Rows decode to
    /// the same `PieceDef` the sim runs.
    PieceDefs {
        total: u8,
        first: u8,
        count: u8,
        rows: [PieceDef; PIECE_DEFS_BATCH],
    },
    /// A deployable landed (broadcast — world facts like pieces). The
    /// wire carries address + row + the door's open bit; owner/hp/uh stay
    /// sim-side, so decoded records hold their defaults there.
    DeployPlaced { rec: DeployRec },
    /// One batch of the placed-deployable walk (join sync / resync).
    DeploySync {
        reset: bool,
        recs: [DeployRec; DEPLOY_SYNC_BATCH],
        count: u8,
    },
    /// A deploy or feed request bounced: `reason` is a
    /// `sim_core::deploy::REFUSE_D_*` code.
    DeployRefused { reason: u8 },
    /// Deploy-def rows `first..first+count` of a `total`-row table — the
    /// deployable menu's data, dripped like the piece defs.
    DeployDefs {
        total: u8,
        first: u8,
        count: u8,
        rows: [DeployDef; DEPLOY_DEFS_BATCH],
    },
    /// A structure at the address took damage and is still standing
    /// (broadcast). `deploy` picks the store the address names — the door
    /// in a doorway and the doorway itself share one address. `left` is
    /// the hp remaining, absolute like `Health` and for the same reason:
    /// a client that misses one hit hears the whole truth from the next.
    /// Destruction never arrives here; it arrives as `PieceRemoved` /
    /// `DeployRemoved`, so a client that only learns removals still ends
    /// in the right state.
    StructHit {
        deploy: bool,
        cx: u16,
        cz: u16,
        level: u8,
        loc: u8,
        damage: u16,
        left: u16,
    },
    /// The structure at the address was bought back to full (broadcast).
    /// `StructHit`'s mirror image, down to the leading `deploy` bit and
    /// the row width it selects: `healed` is what the payment restored,
    /// `hp` where the structure now stands. `row` rides along so a client
    /// that has not yet walked this address can place it without waiting.
    PieceRepaired {
        deploy: bool,
        cx: u16,
        cz: u16,
        level: u8,
        loc: u8,
        row: u8,
        healed: u16,
        hp: u16,
    },
    /// A satchel charge is stuck to the structure at the address and will
    /// blow in `fuse` ticks (broadcast). `PieceRepaired`'s address, field
    /// for field, so a client already drawing hits and mends on a wall
    /// learns no new layout to draw a countdown on one.
    ///
    /// `fuse` is what remains, not the tick it fires on: a client that
    /// joined mid-fuse has no shared tick origin to subtract from. It is
    /// never zero — a zero-fuse charge is refused at bake — so a zero here
    /// is forged and refuses at both ends.
    ChargePlaced {
        deploy: bool,
        cx: u16,
        cz: u16,
        level: u8,
        loc: u8,
        row: u8,
        fuse: u16,
    },
    /// Decay or a raid removed the piece at the address (broadcast;
    /// in-progress piece walks restart server-side).
    PieceRemoved {
        cx: u16,
        cz: u16,
        level: u8,
        loc: u8,
    },
    /// Decay (or its supporting piece's removal) removed the deployable
    /// at the address.
    DeployRemoved {
        cx: u16,
        cz: u16,
        level: u8,
        loc: u8,
    },
    /// The door at the address changed state (broadcast — a door is a
    /// world fact like the piece it sits in). All three bits are the
    /// state after the action, never a delta: a client that missed one
    /// hears the truth from the next one.
    ///
    /// `has_lock` says a code lock is bolted on and `locked` says it is
    /// armed; the two are separate because "bare", "bolted but open to
    /// all" and "shut" are three different prompts and a client that
    /// could not tell the first two apart would offer a keypad at a door
    /// with no keypad (lock v1).
    ///
    /// **Who the lock remembers is not on the wire, and neither is the
    /// code.** The client presses and learns from the outcome, which is
    /// `DESIGN.md` §5.6's own mispredict example — the alternative is a
    /// broadcast that differs per recipient.
    Door {
        cx: u16,
        cz: u16,
        level: u8,
        loc: u8,
        open: bool,
        locked: bool,
        has_lock: bool,
    },
    /// Somebody knocked on the door at the address (broadcast). The one
    /// event in this enum whose *whole* purpose is to be heard by people
    /// it is not addressed to: a knock is what a locked-out player has
    /// instead of a door (`reference/DOORS.md` §4).
    ///
    /// No state and no reason field — it says somebody is at the door and
    /// deliberately not whether they were refused for want of a code, a
    /// list or a lock.
    Knock {
        cx: u16,
        cz: u16,
        level: u8,
        loc: u8,
        by: u32,
    },
    /// A correct code: the lock at the address now remembers **you**, at
    /// `grant` (`sim_core::lock::GRANT_*`). Own-fact, like `Health` — a
    /// client learns its own rights and nothing about anyone else's, so
    /// the remembered list never leaves the sim as a list.
    Auth {
        cx: u16,
        cz: u16,
        level: u8,
        loc: u8,
        grant: u8,
    },
    /// The oven at the address is now `lit` (broadcast). Absolute, never
    /// a delta, for `Door`'s reason. `by` is the hand that pressed, or 0
    /// when the fire ran dry on its own.
    Oven {
        cx: u16,
        cz: u16,
        level: u8,
        lit: bool,
        by: u32,
    },
    /// An arrow left `shooter`'s bow, aimed at (`yaw`, `pitch`), flying at
    /// `speed_mmpt` and falling `drop_mmpt2` a tick (broadcast).
    ///
    /// No origin and no item — the client reconstructs the first from the
    /// shooter's snapshot and does not need the second to draw a shaft.
    Shot {
        shooter: u32,
        yaw: u16,
        pitch: u8,
        speed_mmpt: u16,
        drop_mmpt2: u16,
    },
    /// An arrow stopped here, on this kind of surface (broadcast, wire
    /// v44). Position in the entity lane's quanta — 3 cm in x/z, 1 cm in
    /// y — and `surf` is a `sim_core::ranged::SURF_*`.
    ///
    /// No shooter and no item, `Shot`'s omissions for `Shot`'s reason: a
    /// mark in the world is the same mark whoever made it, and the client
    /// that wants to know whose arrow it was already heard the shot.
    Impact { qx: i32, qy: i32, qz: i32, surf: u8 },
    /// A body swung (wire v47). The swinger's entity id and nothing else:
    /// the receiver already knows where that body is and which way it
    /// faces, and what it could not know is that the arm moved, because a
    /// client never receives another player's input frame.
    ///
    /// Outcome-free on purpose — it fires on a whiff as well as a hit, and
    /// the whiff is the commoner of the two. `EV_HIT` is the hit fact and
    /// is unicast to the attacker.
    Swing { swinger: u32 },
    /// The feed ack: the hearth's stock rows after the transfer, aligned
    /// to the baked upkeep-material list (item index, units).
    Stock {
        cx: u16,
        cz: u16,
        level: u8,
        rows: [(u16, u32); HEARTH_STOCK_ROWS],
        count: u8,
    },
    /// One chat line, relayed (`chat.rs`). `from` is the speaker's player
    /// id — there are no names yet, and the sender's own line comes back
    /// through this same path, so the echo is the delivery receipt.
    /// `global` is which channel it arrived on; the server has already
    /// decided this recipient is entitled to hear it, so the bit is a
    /// label for the client to render, never a filter for it to apply.
    Chat {
        from: u32,
        global: bool,
        text: ChatText,
    },
    /// Your swing landed on `victim` for `damage` (combat.rs). The
    /// attacker's fact and the attacker's alone — a hitmarker, not a
    /// health readout; the victim's own `Health` is the truth about the
    /// victim, and it goes only to them.
    Hit { victim: u32, damage: u16 },
    /// Your hp after something changed it, and the max it is measured
    /// against. Absolute, never a delta: a client that misses one hears
    /// the whole truth from the next, exactly like `Door`.
    Health { hp: u16, max: u16 },
    /// Your food and water, and the ceilings they are measured against.
    /// Absolute for exactly `Health`'s reason — a client that misses one
    /// hears the whole truth from the next, so no client-side meter can
    /// drift away from the sim's (survival.rs).
    Vitals {
        food: u16,
        water: u16,
        max_food: u16,
        max_water: u16,
    },
    /// You ate the item in `slot`. Own-fact: the acknowledgement the HUD
    /// plays the ramp off, and the reason a client never has to guess
    /// whether a press landed.
    Consumed { item: u16, slot: u8 },
    /// The eat did nothing, and why (`sim_core::survival::REFUSE_C_*`).
    /// A refused *drink* arrives here too — one refusal channel for the
    /// whole survival module.
    ConsumeRefused { reason: u8 },
    /// You drank, `water` units went into the meter, and `hp_cost` came
    /// out of your health for it. Own-fact, and the pair is what makes it
    /// worth a subtype: the meter and the hp both travel absolutely on
    /// their own events, so this one exists to say *why* they moved
    /// together (survival.rs).
    Drank { water: u16, hp_cost: u16 },
    /// `victim` was killed by `killer` — broadcast, because a death is a
    /// world fact like a placement, and the kill feed the reference frames
    /// carry bottom-left is built from exactly this.
    ///
    /// Widened in v16 to carry what the death screen is made of:
    /// `cause` is a `sim_core::world::DEATH_BY_*` code, `item` the weapon
    /// in the killer's hand (`NO_ITEM` when the world did it), `range_cm`
    /// how far the blow landed from. **Still no position** — that is not an
    /// omission but the rule ALPHA.md §1 states outright ("no map
    /// position"): a death that told you where you fell would hand every
    /// raider a map pin to the base they just cleared, and hand the corpse
    /// one back. Who, with what, from how far. Never where.
    ///
    /// Broadcast rather than own-fact for the same reason it always was —
    /// a kill feed reports kills nobody saw — and the extra three fields
    /// broadcast with it because they are the feed's content too: "killed
    /// you with a rock from 2 m" is the line, for everyone.
    Death {
        victim: u32,
        killer: u32,
        cause: u8,
        item: u16,
        range_cm: u16,
    },
    /// You woke up: `on_bag` is true if one of your own sleeping bags
    /// answered, false if the beach ring did. Own-fact, and the one message
    /// that closes the death screen.
    ///
    /// Carries no position on purpose — the snapshot has always carried
    /// that and still does. What it carries is the thing a coordinate
    /// cannot say: *which anchor answered*. Ask for a bag inside its
    /// five-minute cooldown and the ring answers instead (world.rs), and a
    /// player who is not told that has no way to learn it except by
    /// looking around at a beach they did not choose.
    Respawn { on_bag: bool },
    /// A death backpack landed at a world position — broadcast, because a
    /// bag on the ground is a world fact like a placement. What is inside
    /// is deliberately absent: v0 has no container UI, the take is
    /// all-that-fits, and shipping every stack to every client would put
    /// the whole shard's loot on every wire for a thing most of them will
    /// never reach.
    BagDropped { id: u32, qx: i32, qy: i32, qz: i32 },
    /// One batch of the standing-bag walk (join sync / event-lane
    /// resync). `reset` clears the client's bag set first.
    BagSync {
        reset: bool,
        recs: [WireBag; BAG_SYNC_BATCH],
        count: u8,
    },
    /// The contents of the container this client has open — the answer to
    /// `ActionMsg::Container`, and the only message on the lane whose
    /// audience is a single client by design rather than by relevance.
    ///
    /// `kind` and `cont` are the same pair the action carried, echoed back
    /// so a client can never apply a batch to the wrong panel: two opens
    /// in flight, a close that crossed an open, or a container that went
    /// away and came back at the same address all read as a mismatch the
    /// client can drop, instead of as slots landing in a box the player
    /// already walked away from.
    ///
    /// `kind == CONT_SELF` is the **close**: nothing is open, the panel
    /// shuts. It arrives unasked-for whenever the server can no longer
    /// prove the opener is in reach of what they opened — walked away,
    /// bag looted out from under them, box raided down — so a panel never
    /// outlives the reach the move verb will judge against.
    ///
    /// `reset` says "this is the whole container, forget what you had";
    /// without it the batch is a diff of the slots that changed since the
    /// last one. An emptied slot crosses as a real change (item 0,
    /// count 0), so a diff can say "that is gone" and not only "that is
    /// new".
    ContSync {
        kind: u8,
        cont: u32,
        reset: bool,
        slots: [InvSlot; CONT_SYNC_BATCH],
        count: u8,
    },
    /// The bag is gone, and why (`sim_core::backpack::BAG_GONE_*`):
    /// despawned, emptied by a take, or evicted by a full store. The
    /// reason is on the wire so the client can tell "someone got there
    /// first" from "you were too slow" — the same information the kill
    /// feed gives about a death.
    BagRemoved { id: u32, why: u8 },
    /// A move landed, own-fact (`inventory.rs`). The address it landed on,
    /// how many moved, and the item that left the source slot.
    ///
    /// The item is the reconcile hook and the reason this is not just an
    /// "ok". A move here is all-or-nothing, so address + count is the
    /// entire diff — *provided* the client's picture of the source slot
    /// was right. `item` is how it finds out it was not: an id it did not
    /// predict means the container drifted and the panel redraws, instead
    /// of the client carrying a divergence forever the way a silently
    /// partial move would leave one.
    Moved {
        from_kind: u8,
        from_slot: u8,
        to_kind: u8,
        to_slot: u8,
        count: u16,
        item: u16,
    },
    /// A move was refused, and why (`sim_core::inventory::REFUSE_M_*`).
    ///
    /// The address rides along and that is the whole point: with it the
    /// client rolls back exactly the drag it predicted; without it the
    /// only safe response to a refusal is to resync a container. This is
    /// also the message that exists so the *other* answer never has to be
    /// given — the reference shipped this failure as the server dropping
    /// the client three times in half an hour (`inventory.rs`).
    MoveRefused {
        reason: u8,
        from_kind: u8,
        from_slot: u8,
        to_kind: u8,
        to_slot: u8,
    },
}

fn begin(buf: &mut [u8], subtype: u32) -> Result<BitWriter<'_>, WireError> {
    let mut w = BitWriter::new(buf);
    w.write(KIND_EVENT, KIND_BITS)?;
    w.write(subtype, SUB_BITS)?;
    Ok(w)
}

pub fn encode_event_gather(item: u16, added: u16, buf: &mut [u8]) -> Result<usize, WireError> {
    let mut w = begin(buf, SUB_GATHER)?;
    w.write(item as u32, 16)?;
    w.write(added as u32, 16)?;
    Ok(w.finish())
}

/// The gather refusal (wire v42). `reason` is a `gather::REFUSE_G_*` code:
/// zero is reserved as "no reason" and refused here exactly as
/// `encode_event_consume_refused` refuses its own zero; anything past the
/// sim's ledger is a bug at this end and refused the same way. `item` is
/// unbounded 16 bits on purpose — `NO_ITEM` (0xFFFF) is a live value
/// meaning bare hands, so the field carries the whole `u16` domain the
/// inventory already speaks.
pub fn encode_event_gather_refused(
    item: u16,
    reason: u8,
    buf: &mut [u8],
) -> Result<usize, WireError> {
    if reason == 0 || (reason as u32) > sim_core::gather::REFUSE_G_MAX {
        return Err(WireError::Range);
    }
    let mut w = begin(buf, SUB_GATHER_REFUSED)?;
    w.write(item as u32, 16)?;
    w.write(reason as u32, REFUSE_G_BITS)?;
    Ok(w.finish())
}

/// `slots` must be non-empty and within `INV_SLOTS` rows/indices — an
/// empty update is a server bug, not a message.
pub fn encode_event_inv(slots: &[InvSlot], buf: &mut [u8]) -> Result<usize, WireError> {
    if slots.is_empty() || slots.len() > INV_SLOTS {
        return Err(WireError::Cap);
    }
    let mut w = begin(buf, SUB_INV)?;
    w.write(slots.len() as u32, INV_COUNT_BITS)?;
    for s in slots {
        if s.slot as usize >= INV_SLOTS {
            return Err(WireError::Range);
        }
        w.write(s.slot as u32, INV_SLOT_BITS)?;
        w.write(s.stack.item as u32, 16)?;
        w.write(s.stack.count as u32, 16)?;
        w.write(s.stack.cond as u32, 16)?;
    }
    Ok(w.finish())
}

pub fn encode_event_slot_change(
    harvested: bool,
    cx: u16,
    cz: u16,
    buf: &mut [u8],
) -> Result<usize, WireError> {
    let sub = if harvested {
        SUB_SLOT_HARVESTED
    } else {
        SUB_SLOT_RESPAWNED
    };
    let mut w = begin(buf, sub)?;
    w.write(cx as u32, 16)?;
    w.write(cz as u32, 16)?;
    Ok(w.finish())
}

/// A batch may be empty only with `reset` — the "your set is now empty"
/// resync message; an empty non-reset batch says nothing.
pub fn encode_event_slot_sync(
    reset: bool,
    cells: &[(u16, u16)],
    buf: &mut [u8],
) -> Result<usize, WireError> {
    if cells.len() > SLOT_SYNC_BATCH || (cells.is_empty() && !reset) {
        return Err(WireError::Cap);
    }
    let mut w = begin(buf, SUB_SLOT_SYNC)?;
    w.write_bit(reset)?;
    w.write(cells.len() as u32, SYNC_COUNT_BITS)?;
    for &(cx, cz) in cells {
        w.write(cx as u32, 16)?;
        w.write(cz as u32, 16)?;
    }
    Ok(w.finish())
}

/// Encode up to `CATALOG_BATCH` names starting at `first`. Returns the
/// byte length and how many names rode along; the caller's cursor
/// advances by that count.
pub fn encode_event_catalog(
    catalog: &ItemCatalog,
    first: usize,
    buf: &mut [u8],
) -> Result<(usize, usize), WireError> {
    let total = catalog.count as usize;
    if total > MAX_ITEM_DEFS || first >= total {
        return Err(WireError::Range);
    }
    let count = CATALOG_BATCH.min(total - first);
    let mut w = begin(buf, SUB_CATALOG)?;
    w.write(total as u32, CATALOG_TOTAL_BITS)?;
    w.write(first as u32, CATALOG_TOTAL_BITS)?;
    w.write(count as u32, CATALOG_COUNT_BITS)?;
    for idx in first..first + count {
        let name = catalog.name(idx);
        if name.is_empty() || name.len() > MAX_ITEM_NAME_BYTES {
            return Err(WireError::Range);
        }
        w.write(name.len() as u32, NAME_LEN_BITS)?;
        for &b in name {
            w.write(b as u32, 8)?;
        }
        w.write(catalog.cond_max(idx) as u32, 16)?;
    }
    Ok((w.finish(), count))
}

pub fn encode_event_weak_mark(
    cx: u16,
    cz: u16,
    mark8: u8,
    weak_hit: bool,
    buf: &mut [u8],
) -> Result<usize, WireError> {
    let mut w = begin(buf, SUB_WEAK_MARK)?;
    w.write(cx as u32, 16)?;
    w.write(cz as u32, 16)?;
    w.write(mark8 as u32, 8)?;
    w.write_bit(weak_hit)?;
    Ok(w.finish())
}

/// `jobs` is the dense live prefix of a player's queue (empty ⇒ "queue
/// cleared"). Refuses a job the wire's widths can't carry: a dead job
/// (`remaining == 0`), a remaining over u8, or a recipe outside the table.
pub fn encode_event_craft_q(
    jobs: &[CraftJob],
    eta_ticks: u16,
    buf: &mut [u8],
) -> Result<usize, WireError> {
    if jobs.len() > CRAFT_QUEUE {
        return Err(WireError::Cap);
    }
    let mut w = begin(buf, SUB_CRAFT_Q)?;
    w.write(jobs.len() as u32, CRAFT_Q_COUNT_BITS)?;
    for j in jobs {
        if j.remaining == 0 || j.remaining > u8::MAX as u16 || j.recipe as usize >= MAX_RECIPES {
            return Err(WireError::Range);
        }
        w.write(j.recipe as u32, 8)?;
        w.write(j.remaining as u32, 8)?;
    }
    w.write(eta_ticks as u32, 16)?;
    Ok(w.finish())
}

pub fn encode_event_craft_done(item: u16, added: u16, buf: &mut [u8]) -> Result<usize, WireError> {
    let mut w = begin(buf, SUB_CRAFT_DONE)?;
    w.write(item as u32, 16)?;
    w.write(added as u32, 16)?;
    Ok(w.finish())
}

pub fn encode_event_research(recipe: u16, cost: u16, buf: &mut [u8]) -> Result<usize, WireError> {
    if recipe as usize >= MAX_RECIPES {
        return Err(WireError::Range);
    }
    let mut w = begin(buf, SUB_RESEARCH)?;
    w.write(recipe as u32, 16)?;
    w.write(cost as u32, 16)?;
    Ok(w.finish())
}

pub fn encode_event_research_refused(reason: u8, buf: &mut [u8]) -> Result<usize, WireError> {
    if reason as u32 > sim_core::research::REFUSE_R_MAX {
        return Err(WireError::Range);
    }
    let mut w = begin(buf, SUB_RESEARCH_REFUSED)?;
    w.write(reason as u32, RESEARCH_REFUSE_BITS)?;
    Ok(w.finish())
}

pub fn encode_event_known(mask: u64, buf: &mut [u8]) -> Result<usize, WireError> {
    let mut w = begin(buf, SUB_KNOWN)?;
    // Two halves rather than a 64-bit write: `BitWriter::write` takes a
    // `u32`, and splitting here keeps the one place that knows the mask is
    // wider than the writer's word next to the one place that reassembles
    // it.
    w.write((mask & 0xFFFF_FFFF) as u32, 32)?;
    w.write((mask >> 32) as u32, 32)?;
    Ok(w.finish())
}

/// Your own bags, whole (wire v42). An **empty list is legal and load-
/// bearing**: it is how a client is told it has no bag left to wake on,
/// which is the state the death screen changes shape for. Every other
/// batch encoder in this file refuses an empty body because emptiness
/// there means "nothing to say"; here it is the thing being said.
pub fn encode_event_bags(bags: &[BagAnchor], buf: &mut [u8]) -> Result<usize, WireError> {
    if bags.len() > BAG_CAP {
        return Err(WireError::Cap);
    }
    let mut w = begin(buf, SUB_BAGS)?;
    w.write(bags.len() as u32, BAGS_COUNT_BITS)?;
    for b in bags {
        if b.cx as usize >= MAX_BUILD_COORD
            || b.cz as usize >= MAX_BUILD_COORD
            || b.level as usize >= MAX_BUILD_LEVELS
        {
            return Err(WireError::Range);
        }
        w.write(b.cx as u32, BUILD_CELL_BITS)?;
        w.write(b.cz as u32, BUILD_CELL_BITS)?;
        w.write(b.level as u32, BUILD_LEVEL_BITS)?;
        w.write_bit(b.ready)?;
    }
    Ok(w.finish())
}

pub fn encode_event_craft_refused(reason: u8, buf: &mut [u8]) -> Result<usize, WireError> {
    let mut w = begin(buf, SUB_CRAFT_REFUSED)?;
    w.write(reason as u32, 8)?;
    Ok(w.finish())
}

/// Encode up to `RECIPE_BATCH` baked recipe rows starting at `first`.
/// Returns the byte length and how many rows rode along. Row shapes the
/// bake refuses (zero ticks, zero output, too many inputs) refuse here
/// too — this encoder only ever sees baked tables.
pub fn encode_event_recipes(
    cc: &CraftContent,
    first: usize,
    buf: &mut [u8],
) -> Result<(usize, usize), WireError> {
    let total = cc.recipe_count as usize;
    if total > MAX_RECIPES || first >= total {
        return Err(WireError::Range);
    }
    let count = RECIPE_BATCH.min(total - first);
    let mut w = begin(buf, SUB_RECIPES)?;
    w.write(total as u32, RECIPE_TOTAL_BITS)?;
    w.write(first as u32, RECIPE_TOTAL_BITS)?;
    w.write(count as u32, RECIPE_COUNT_BITS)?;
    for def in cc.recipes[first..first + count].iter() {
        if def.ticks == 0
            || def.ticks >= (1 << RECIPE_TICKS_BITS)
            || def.out_count == 0
            || def.out_count > u8::MAX as u16
            || def.station > STATION_MAX
            || def.n_inputs == 0
            || def.n_inputs as usize > MAX_RECIPE_INPUTS
        {
            return Err(WireError::Range);
        }
        w.write(def.output as u32, 16)?;
        w.write(def.out_count as u32, 8)?;
        w.write(def.ticks, RECIPE_TICKS_BITS)?;
        w.write(def.station as u32, STATION_BITS)?;
        // One bit, and it has to cross: a client that did not know a
        // recipe was locked would offer it, take the press, and be told
        // "you have not learned this" by a server the player cannot see.
        // The craft panel greys it instead (research v0).
        w.write_bit(def.blueprint)?;
        w.write(def.n_inputs as u32, N_INPUTS_BITS)?;
        for &(item, per) in def.inputs.iter().take(def.n_inputs as usize) {
            w.write(item as u32, 16)?;
            w.write(per as u32, 16)?;
        }
    }
    Ok((w.finish(), count))
}

/// Encode up to `RESEARCH_BATCH` baked research rows starting at
/// `first` (tech tree v0) — `encode_event_recipes`' shape on the
/// research table. A recipe index rides 8 bits (`MAX_RECIPES` is 64);
/// `requires` rides the same width with `0xFF` as the wire's spelling of
/// [`sim_core::research::NO_RECIPE`], which no live recipe can reach.
pub fn encode_event_research_rows(
    rc: &sim_core::research::ResearchContent,
    first: usize,
    buf: &mut [u8],
) -> Result<(usize, usize), WireError> {
    let total = rc.row_count as usize;
    if total > MAX_RESEARCH_ROWS || first >= total {
        return Err(WireError::Range);
    }
    let count = RESEARCH_BATCH.min(total - first);
    let mut w = begin(buf, SUB_RESEARCH_ROWS)?;
    w.write(total as u32, RESEARCH_TOTAL_BITS)?;
    w.write(first as u32, RESEARCH_TOTAL_BITS)?;
    w.write(count as u32, RESEARCH_COUNT_BITS)?;
    w.write(rc.coin as u32, 16)?;
    for row in rc.rows[first..first + count].iter() {
        let requires_ok = row.requires == NO_RECIPE || (row.requires as usize) < MAX_RECIPES;
        if (row.recipe as usize) >= MAX_RECIPES || !requires_ok {
            return Err(WireError::Range);
        }
        w.write(row.item as u32, 16)?;
        w.write(row.recipe as u32, 8)?;
        w.write(row.cost as u32, 16)?;
        let req = if row.requires == NO_RECIPE {
            0xFF
        } else {
            row.requires as u32
        };
        w.write(req, 8)?;
    }
    Ok((w.finish(), count))
}

/// One placed-piece record on the wire: 38 bits, shared by the placed
/// broadcast and the sync batches. Refuses an address outside the grid or
/// a row outside the def table — this encoder only ever sees sim records.
/// The trailing bit is the soft side's facing (hard/soft v0, wire v39):
/// the client labels the side a player is looking at, so the bit rides
/// every record the way a door's open bit does.
fn write_piece_rec(w: &mut BitWriter, rec: &PieceRec) -> Result<(), WireError> {
    if rec.cx as usize >= MAX_BUILD_COORD
        || rec.cz as usize >= MAX_BUILD_COORD
        || rec.level as usize >= MAX_BUILD_LEVELS
        || rec.loc > loc_max(false)
        || rec.row as usize >= MAX_PIECE_DEFS
        || rec.facing > 1
    {
        return Err(WireError::Range);
    }
    w.write(rec.cx as u32, BUILD_CELL_BITS)?;
    w.write(rec.cz as u32, BUILD_CELL_BITS)?;
    w.write(rec.level as u32, BUILD_LEVEL_BITS)?;
    w.write(rec.loc as u32, BUILD_LOC_BITS)?;
    w.write(rec.row as u32, PIECE_ROW_BITS)?;
    w.write_bit(rec.facing != 0)?;
    // The damage band (wire v44). `dmg` is a wire field the store does not
    // maintain — `PieceRec::dmg` says so — so a caller that forgets to fill
    // it sends 0, which draws an untouched wall. `server::core` fills it at
    // the one place it builds these; `tests/protocol_golden.rs` §dmg holds
    // the round trip.
    w.write((rec.dmg & (DMG_BANDS - 1)) as u32, DMG_BAND_BITS)?;
    // The column's plate (build plate v1, wire v49), biased into
    // `PLATE_BITS`. Unlike `dmg` this IS sim state and the store maintains
    // it — a record that arrives with the wrong plate draws the whole base
    // on the wrong floor, which is why it rides every record rather than a
    // per-column lane: a column's plate arrives with its pieces and leaves
    // with its last one, so there is no third message to forget.
    w.write(
        (rec.plate as i32 + PLATE_BIAS).clamp(0, (1 << PLATE_BITS) - 1) as u32,
        PLATE_BITS,
    )?;
    Ok(())
}

fn read_piece_rec(r: &mut BitReader) -> Result<PieceRec, WireError> {
    let rec = PieceRec {
        cx: r.read(BUILD_CELL_BITS)? as u16,
        cz: r.read(BUILD_CELL_BITS)? as u16,
        level: r.read(BUILD_LEVEL_BITS)? as u8,
        loc: r.read(BUILD_LOC_BITS)? as u8,
        row: r.read(PIECE_ROW_BITS)? as u8,
        ..PieceRec::default()
    };
    let rec = PieceRec {
        facing: r.read_bit()? as u8,
        // Every one of the eight values `DMG_BAND_BITS` can carry is legal,
        // so this needs no range check — the width is the check.
        dmg: r.read(DMG_BAND_BITS)? as u8,
        plate: (r.read(PLATE_BITS)? as i32 - PLATE_BIAS) as i8,
        ..rec
    };
    // Coord/level/facing widths are exact; the row — and, since v40's
    // widening, the loc — can be forged.
    if rec.row as usize >= MAX_PIECE_DEFS || rec.loc > loc_max(false) {
        return Err(WireError::Malformed);
    }
    Ok(rec)
}

pub fn encode_event_piece_placed(rec: &PieceRec, buf: &mut [u8]) -> Result<usize, WireError> {
    let mut w = begin(buf, SUB_PIECE_PLACED)?;
    write_piece_rec(&mut w, rec)?;
    Ok(w.finish())
}

/// A batch may be empty only with `reset` — the "your piece set is now
/// empty" resync message, same contract as the slot sync.
pub fn encode_event_piece_sync(
    reset: bool,
    recs: &[PieceRec],
    buf: &mut [u8],
) -> Result<usize, WireError> {
    if recs.len() > PIECE_SYNC_BATCH || (recs.is_empty() && !reset) {
        return Err(WireError::Cap);
    }
    let mut w = begin(buf, SUB_PIECE_SYNC)?;
    w.write_bit(reset)?;
    w.write(recs.len() as u32, PIECE_SYNC_COUNT_BITS)?;
    for rec in recs {
        write_piece_rec(&mut w, rec)?;
    }
    Ok(w.finish())
}

pub fn encode_event_build_refused(reason: u8, buf: &mut [u8]) -> Result<usize, WireError> {
    let mut w = begin(buf, SUB_BUILD_REFUSED)?;
    w.write(reason as u32, REFUSE_B_BITS)?;
    Ok(w.finish())
}

/// A piece bought back to full: the address, how much life the payment
/// restored, and what it stands at now.
///
/// `hp` rides along rather than being implied by "full", because a client
/// that has not received the piece-defs drip for this row does not know
/// what full is — the same reason `StructHit` sends `left` instead of a
/// delta alone. `healed == 0` is refused at both ends: the sim never
/// announces a repair that bought nothing, and a decoder that accepted one
/// would let a forged frame zero a client's hp mirror for free.
// Its mirror `encode_event_struct_hit` carries the same allow for the same
// reason: an address is four fields before any payload, and bundling them
// into a struct here would put a second definition of "an address" beside
// the one `PieceRec` already is.
#[allow(clippy::too_many_arguments)]
pub fn encode_event_piece_repaired(
    deploy: bool,
    cx: u16,
    cz: u16,
    level: u8,
    loc: u8,
    row: u8,
    healed: u16,
    hp: u16,
    buf: &mut [u8],
) -> Result<usize, WireError> {
    // `encode_event_struct_hit`'s row-width rule, verbatim: a deployable
    // row is four bits and a piece row eight, so the bit that names the
    // store also sizes the field after it.
    let row_bits = if deploy {
        DEPLOY_ROW_BITS
    } else {
        PIECE_ROW_BITS
    };
    if cx as usize >= MAX_BUILD_COORD
        || cz as usize >= MAX_BUILD_COORD
        || level as usize >= MAX_BUILD_LEVELS
        || loc > loc_max(deploy)
        || (row as u32) >= (1 << row_bits)
        || healed == 0
        || hp == 0
        || healed > hp
    {
        return Err(WireError::Range);
    }
    let mut w = begin(buf, SUB_PIECE_REPAIRED)?;
    w.write_bit(deploy)?;
    w.write(cx as u32, BUILD_CELL_BITS)?;
    w.write(cz as u32, BUILD_CELL_BITS)?;
    w.write(level as u32, BUILD_LEVEL_BITS)?;
    w.write(loc as u32, BUILD_LOC_BITS)?;
    w.write(row as u32, row_bits)?;
    w.write(healed as u32, 16)?;
    w.write(hp as u32, 16)?;
    Ok(w.finish())
}

/// Encode a planted charge. `encode_event_piece_repaired`'s address
/// prologue verbatim — including the row-width rule the store bit selects
/// — with one 16-bit fuse where its two payload fields sit.
///
/// `fuse == 0` is refused at both ends for `healed == 0`'s reason: the sim
/// cannot announce a charge that blows the instant it is planted (the bake
/// refuses a zero fuse), so a zero on the wire is forged, and a client
/// that accepted one would draw a countdown that never counts.
#[allow(clippy::too_many_arguments)]
pub fn encode_event_charge_placed(
    deploy: bool,
    cx: u16,
    cz: u16,
    level: u8,
    loc: u8,
    row: u8,
    fuse: u16,
    buf: &mut [u8],
) -> Result<usize, WireError> {
    let row_bits = if deploy {
        DEPLOY_ROW_BITS
    } else {
        PIECE_ROW_BITS
    };
    if cx as usize >= MAX_BUILD_COORD
        || cz as usize >= MAX_BUILD_COORD
        || level as usize >= MAX_BUILD_LEVELS
        || loc > loc_max(deploy)
        || (row as u32) >= (1 << row_bits)
        || fuse == 0
    {
        return Err(WireError::Range);
    }
    let mut w = begin(buf, SUB_CHARGE_PLACED)?;
    w.write_bit(deploy)?;
    w.write(cx as u32, BUILD_CELL_BITS)?;
    w.write(cz as u32, BUILD_CELL_BITS)?;
    w.write(level as u32, BUILD_LEVEL_BITS)?;
    w.write(loc as u32, BUILD_LOC_BITS)?;
    w.write(row as u32, row_bits)?;
    w.write(fuse as u32, 16)?;
    Ok(w.finish())
}

/// Encode up to `PIECE_DEFS_BATCH` baked piece rows starting at `first`.
/// Returns the byte length and how many rows rode along. Row shapes the
/// bake refuses (hp 0, too many costs) refuse here too.
pub fn encode_event_piece_defs(
    bc: &BuildContent,
    first: usize,
    buf: &mut [u8],
) -> Result<(usize, usize), WireError> {
    let total = bc.piece_count as usize;
    if total > MAX_PIECE_DEFS || first >= total {
        return Err(WireError::Range);
    }
    let count = PIECE_DEFS_BATCH.min(total - first);
    let mut w = begin(buf, SUB_PIECE_DEFS)?;
    w.write(total as u32, PIECE_DEFS_TOTAL_BITS)?;
    w.write(first as u32, PIECE_DEFS_TOTAL_BITS)?;
    w.write(count as u32, PIECE_DEFS_COUNT_BITS)?;
    for def in bc.pieces[first..first + count].iter() {
        // `SHAPE_TRI_ROOF` is the top code (the triangles, wire v40).
        if def.shape > SHAPE_TRI_ROOF
            || def.material > MAT_METAL
            || def.hp == 0
            || def.n_costs == 0
            || def.n_costs as usize > MAX_PIECE_COSTS
        {
            return Err(WireError::Range);
        }
        w.write(def.shape as u32, SHAPE_BITS)?;
        w.write(def.material as u32, MATERIAL_BITS)?;
        w.write(def.hp as u32, 16)?;
        w.write(def.n_costs as u32, N_COSTS_BITS)?;
        for &(item, units) in def.costs.iter().take(def.n_costs as usize) {
            w.write(item as u32, 16)?;
            w.write(units as u32, 16)?;
        }
    }
    Ok((w.finish(), count))
}

/// One placed-deployable record on the wire: 32 bits, shared by the
/// placed broadcast and the sync batches. Every width is exact, so only
/// sim-impossible addresses need refusing at encode. The trailing three
/// bits are open, locked and has-lock state. Open is the door's alone;
/// locked and has-lock ride for every lockable archetype — a door or a
/// box (`sim_core::deploy::lockable`) — and all three are 0 for the rest,
/// so they cost three bits and save a second lane for the join walk (a
/// client that walked in must see which doors stand open, and which
/// leaves stand locked or carry a keypad at all).
fn write_deploy_rec(w: &mut BitWriter, rec: &DeployRec) -> Result<(), WireError> {
    if rec.cx as usize >= MAX_BUILD_COORD
        || rec.cz as usize >= MAX_BUILD_COORD
        || rec.level as usize >= MAX_BUILD_LEVELS
        // Still the straight-edge bound: a deployable never sits on a
        // triangle or a diagonal (v40 widened the FIELD, not this store).
        || rec.loc > LOC_EDGE_ZLO
        || rec.row as usize >= MAX_DEPLOY_DEFS
    {
        return Err(WireError::Range);
    }
    w.write(rec.cx as u32, BUILD_CELL_BITS)?;
    w.write(rec.cz as u32, BUILD_CELL_BITS)?;
    w.write(rec.level as u32, BUILD_LEVEL_BITS)?;
    w.write(rec.loc as u32, BUILD_LOC_BITS)?;
    w.write(rec.row as u32, DEPLOY_ROW_BITS)?;
    w.write_bit(rec.open)?;
    w.write_bit(rec.locked)?;
    w.write_bit(rec.has_lock)?;
    // The damage band (wire v44) — `write_piece_rec`'s note applies here.
    w.write((rec.dmg & (DMG_BANDS - 1)) as u32, DMG_BAND_BITS)?;
    // **No plate here, deliberately** (build plate v1). A deployable stands
    // on a piece or on bare ground, and in the first case the piece record
    // for its own column already carries the plate — so a second copy on
    // this record would be a field two messages could disagree about, for a
    // column the renderer can look up in the mirror it already keeps
    // (`render/structures.rs` reads `ClientCore::cols`). The ordering hazard
    // that buys — a deploy arriving before its column's pieces — is closed
    // on the render side by treating the plate as redraw state, the way the
    // door's own open bit is.
    Ok(())
}

fn read_deploy_rec(r: &mut BitReader) -> Result<DeployRec, WireError> {
    let rec = DeployRec {
        cx: r.read(BUILD_CELL_BITS)? as u16,
        cz: r.read(BUILD_CELL_BITS)? as u16,
        level: r.read(BUILD_LEVEL_BITS)? as u8,
        loc: r.read(BUILD_LOC_BITS)? as u8,
        row: r.read(DEPLOY_ROW_BITS)? as u8,
        open: r.read_bit()?,
        locked: r.read_bit()?,
        has_lock: r.read_bit()?,
        // Width is the range check — see `read_piece_rec`.
        dmg: r.read(DMG_BAND_BITS)? as u8,
        ..DeployRec::default()
    };
    // The loc became forgeable when the field widened for the piece
    // store's triangles (v40); this store never grew.
    if rec.loc > loc_max(true) {
        return Err(WireError::Malformed);
    }
    Ok(rec)
}

pub fn encode_event_deploy_placed(rec: &DeployRec, buf: &mut [u8]) -> Result<usize, WireError> {
    let mut w = begin(buf, SUB_DEPLOY_PLACED)?;
    write_deploy_rec(&mut w, rec)?;
    Ok(w.finish())
}

/// A batch may be empty only with `reset` — the same contract as the
/// piece sync.
pub fn encode_event_deploy_sync(
    reset: bool,
    recs: &[DeployRec],
    buf: &mut [u8],
) -> Result<usize, WireError> {
    if recs.len() > DEPLOY_SYNC_BATCH || (recs.is_empty() && !reset) {
        return Err(WireError::Cap);
    }
    let mut w = begin(buf, SUB_DEPLOY_SYNC)?;
    w.write_bit(reset)?;
    w.write(recs.len() as u32, DEPLOY_SYNC_COUNT_BITS)?;
    for rec in recs {
        write_deploy_rec(&mut w, rec)?;
    }
    Ok(w.finish())
}

pub fn encode_event_deploy_refused(reason: u8, buf: &mut [u8]) -> Result<usize, WireError> {
    let mut w = begin(buf, SUB_DEPLOY_REFUSED)?;
    w.write(reason as u32, 8)?;
    Ok(w.finish())
}

/// Encode up to `DEPLOY_DEFS_BATCH` baked deployable rows starting at
/// `first`. Returns the byte length and how many rows rode along. Row
/// shapes the bake refuses (hp 0, out-of-range codes) refuse here too.
pub fn encode_event_deploy_defs(
    dc: &DeployContent,
    first: usize,
    buf: &mut [u8],
) -> Result<(usize, usize), WireError> {
    let total = dc.def_count as usize;
    if total > MAX_DEPLOY_DEFS || first >= total {
        return Err(WireError::Range);
    }
    let count = DEPLOY_DEFS_BATCH.min(total - first);
    let mut w = begin(buf, SUB_DEPLOY_DEFS)?;
    w.write(total as u32, DEPLOY_DEFS_TOTAL_BITS)?;
    w.write(first as u32, DEPLOY_DEFS_TOTAL_BITS)?;
    w.write(count as u32, DEPLOY_DEFS_COUNT_BITS)?;
    for def in dc.defs[first..first + count].iter() {
        if def.arch > ARCH_WORKBENCH3 || def.placement > PLACE_DOOR || def.hp == 0 {
            return Err(WireError::Range);
        }
        if def.n_costs as usize > MAX_DEPLOY_COSTS {
            return Err(WireError::Range);
        }
        w.write(def.arch as u32, ARCH_BITS)?;
        w.write(def.placement as u32, PLACEMENT_BITS)?;
        w.write(def.hp as u32, 16)?;
        w.write(def.item as u32, 16)?;
        // The repair price, the same way `SUB_PIECE_DEFS` carries a
        // piece's. Without it a client can quote what a wall costs to mend
        // and not what a door costs, which is the half of the verb a raid
        // actually meets. Zero rows is legal and means unpriced.
        w.write(def.n_costs as u32, DEPLOY_COSTS_BITS)?;
        for &(item, units) in def.costs.iter().take(def.n_costs as usize) {
            w.write(item as u32, 16)?;
            w.write(units as u32, 16)?;
        }
    }
    Ok((w.finish(), count))
}

/// One landed raid swing: the address, what it took, and what is left.
/// `deploy` picks which store the address names.
#[allow(clippy::too_many_arguments)]
pub fn encode_event_struct_hit(
    deploy: bool,
    cx: u16,
    cz: u16,
    level: u8,
    loc: u8,
    row: u8,
    damage: u16,
    left: u16,
    buf: &mut [u8],
) -> Result<usize, WireError> {
    let row_bits = if deploy {
        DEPLOY_ROW_BITS
    } else {
        PIECE_ROW_BITS
    };
    if cx as usize >= MAX_BUILD_COORD
        || cz as usize >= MAX_BUILD_COORD
        || level as usize >= MAX_BUILD_LEVELS
        || loc > loc_max(deploy)
        || (row as u32) >= (1 << row_bits)
        || left == 0
    {
        // `left == 0` is out of range on purpose: a structure at zero hp
        // is a removal, and removals ride their own subtype.
        return Err(WireError::Range);
    }
    let mut w = begin(buf, SUB_STRUCT_HIT)?;
    w.write_bit(deploy)?;
    w.write(cx as u32, BUILD_CELL_BITS)?;
    w.write(cz as u32, BUILD_CELL_BITS)?;
    w.write(level as u32, BUILD_LEVEL_BITS)?;
    w.write(loc as u32, BUILD_LOC_BITS)?;
    w.write(row as u32, row_bits)?;
    w.write(damage as u32, 16)?;
    w.write(left as u32, 16)?;
    Ok(w.finish())
}

/// A decay or raid removal at a grid address — `piece` picks which store.
pub fn encode_event_removed(
    piece: bool,
    cx: u16,
    cz: u16,
    level: u8,
    loc: u8,
    buf: &mut [u8],
) -> Result<usize, WireError> {
    if cx as usize >= MAX_BUILD_COORD
        || cz as usize >= MAX_BUILD_COORD
        || level as usize >= MAX_BUILD_LEVELS
        || loc > loc_max(!piece)
    {
        return Err(WireError::Range);
    }
    let sub = if piece {
        SUB_PIECE_REMOVED
    } else {
        SUB_DEPLOY_REMOVED
    };
    let mut w = begin(buf, sub)?;
    w.write(cx as u32, BUILD_CELL_BITS)?;
    w.write(cz as u32, BUILD_CELL_BITS)?;
    w.write(level as u32, BUILD_LEVEL_BITS)?;
    w.write(loc as u32, BUILD_LOC_BITS)?;
    Ok(w.finish())
}

/// The feed ack: `rows` are the hearth's live stock rows, aligned to the
/// baked upkeep-material list. Empty is legal (a hearth with no priced
/// materials cannot exist, but the width allows the message shape).
pub fn encode_event_stock(
    cx: u16,
    cz: u16,
    level: u8,
    rows: &[(u16, u32)],
    buf: &mut [u8],
) -> Result<usize, WireError> {
    if rows.len() > HEARTH_STOCK_ROWS {
        return Err(WireError::Cap);
    }
    if cx as usize >= MAX_BUILD_COORD
        || cz as usize >= MAX_BUILD_COORD
        || level as usize >= MAX_BUILD_LEVELS
    {
        return Err(WireError::Range);
    }
    let mut w = begin(buf, SUB_STOCK)?;
    w.write(cx as u32, BUILD_CELL_BITS)?;
    w.write(cz as u32, BUILD_CELL_BITS)?;
    w.write(level as u32, BUILD_LEVEL_BITS)?;
    w.write(rows.len() as u32, STOCK_COUNT_BITS)?;
    for &(item, units) in rows {
        w.write(item as u32, 16)?;
        w.write(units, 32)?;
    }
    Ok(w.finish())
}

/// The door at the address is now `open` and `locked` (broadcast).
/// Absolute state, not a toggle: two of these crossing never leave a
/// client inverted, and one lane carries the whole door so a client never
/// holds half of it.
#[allow(clippy::too_many_arguments)]
pub fn encode_event_door(
    cx: u16,
    cz: u16,
    level: u8,
    loc: u8,
    open: bool,
    locked: bool,
    has_lock: bool,
    buf: &mut [u8],
) -> Result<usize, WireError> {
    if !door_addr_ok(cx, cz, level, loc) {
        return Err(WireError::Range);
    }
    let mut w = begin(buf, SUB_DOOR)?;
    w.write(cx as u32, BUILD_CELL_BITS)?;
    w.write(cz as u32, BUILD_CELL_BITS)?;
    w.write(level as u32, BUILD_LEVEL_BITS)?;
    w.write(loc as u32, BUILD_LOC_BITS)?;
    w.write_bit(open)?;
    w.write_bit(locked)?;
    w.write_bit(has_lock)?;
    Ok(w.finish())
}

/// The address every door-lane message shares. One predicate, because
/// three encoders refusing on three copies of the same four comparisons
/// is three chances for one of them to drift.
fn door_addr_ok(cx: u16, cz: u16, level: u8, loc: u8) -> bool {
    (cx as usize) < MAX_BUILD_COORD
        && (cz as usize) < MAX_BUILD_COORD
        && (level as usize) < MAX_BUILD_LEVELS
        && loc <= LOC_EDGE_ZLO
}

/// Somebody knocked on the door at the address (broadcast, lock v1).
pub fn encode_event_knock(
    cx: u16,
    cz: u16,
    level: u8,
    loc: u8,
    by: u32,
    buf: &mut [u8],
) -> Result<usize, WireError> {
    if !door_addr_ok(cx, cz, level, loc) {
        return Err(WireError::Range);
    }
    let mut w = begin(buf, SUB_KNOCK)?;
    w.write(cx as u32, BUILD_CELL_BITS)?;
    w.write(cz as u32, BUILD_CELL_BITS)?;
    w.write(level as u32, BUILD_LEVEL_BITS)?;
    w.write(loc as u32, BUILD_LOC_BITS)?;
    w.write(by, 32)?;
    Ok(w.finish())
}

/// Width of a grant (`sim_core::lock::GRANT_*`): three values in two bits,
/// so the fourth is forgeable and the decoder refuses it.
const LOCK_GRANT_BITS: u32 = 2;
/// Width of the access op (`sim_core::deploy::ACCESS_OP_*`). **Widened
/// 3 → 4 in wire v29**: the hearth's three crew ops joined the lock's six
/// in one space, and three bits hold eight. Seven of the sixteen values
/// are now forgeable, so the decoder range-checks rather than trusting
/// the field.
///
/// **Declared here rather than beside its encoder in `lib.rs`**, which is
/// where the C→S action lane lives, and that is on purpose: the
/// wire-domain gate below reads *this file* for the widths it bounds, so a
/// width declared one module away would be a domain with no gate — which
/// is the exact 2026-08-05 shape that gate exists for.
pub(crate) const ACCESS_OP_BITS: u32 = 4;
/// Width of a four-digit code (0..=9999). A **magnitude**, not a domain —
/// the value is a number a player types, not a member of an enumeration —
/// so it is classified in `MAGNITUDES` below. Fourteen bits leave
/// 10 000..=16 383 forgeable, hence the decode-side check in `lib.rs`.
pub(crate) const LOCK_CODE_BITS: u32 = 14;

/// The lock at the address now remembers the recipient, at `grant`
/// (own-fact, lock v1).
pub fn encode_event_auth(
    cx: u16,
    cz: u16,
    level: u8,
    loc: u8,
    grant: u8,
    buf: &mut [u8],
) -> Result<usize, WireError> {
    if !door_addr_ok(cx, cz, level, loc) || grant > sim_core::lock::GRANT_FULL {
        return Err(WireError::Range);
    }
    let mut w = begin(buf, SUB_AUTH)?;
    w.write(cx as u32, BUILD_CELL_BITS)?;
    w.write(cz as u32, BUILD_CELL_BITS)?;
    w.write(level as u32, BUILD_LEVEL_BITS)?;
    w.write(loc as u32, BUILD_LOC_BITS)?;
    w.write(grant as u32, LOCK_GRANT_BITS)?;
    Ok(w.finish())
}

/// The oven at the address is now `lit` (broadcast) — see
/// `EventMsg::Oven`. No `loc`: an oven is a body deployable and stands at
/// `LOC_PLANE`, so carrying the field would be carrying a constant and
/// inviting a client to believe a fire could be in a doorway.
pub fn encode_event_oven(
    cx: u16,
    cz: u16,
    level: u8,
    lit: bool,
    by: u32,
    buf: &mut [u8],
) -> Result<usize, WireError> {
    if cx as usize >= MAX_BUILD_COORD
        || cz as usize >= MAX_BUILD_COORD
        || level as usize >= MAX_BUILD_LEVELS
    {
        return Err(WireError::Range);
    }
    let mut w = begin(buf, SUB_OVEN)?;
    w.write(cx as u32, BUILD_CELL_BITS)?;
    w.write(cz as u32, BUILD_CELL_BITS)?;
    w.write(level as u32, BUILD_LEVEL_BITS)?;
    w.write_bit(lit)?;
    w.write(by, 32)?;
    Ok(w.finish())
}

/// An arrow left a bow — the tracer's whole input (wire v33).
///
/// The angles ride as **16 and 8 bits, separately** — the same two widths
/// the input frame spends on its way in (`lib.rs`), so a shot goes back
/// out at exactly the precision it was aimed with. Deliberately not one
/// packed word: the sim packs them into `EV_SHOT.b` because a `SimEvent`
/// has only three fields to spend, and a packed word is where a swap of
/// the halves hides. The wire has room, so it unpacks them.
pub fn encode_event_shot(
    shooter: u32,
    yaw: u16,
    pitch: u8,
    speed_mmpt: u16,
    drop_mmpt2: u16,
    buf: &mut [u8],
) -> Result<usize, WireError> {
    // A round with no speed is not a round (`content::validate` refuses
    // one), and a zero here would be a tracer that hangs in the air at the
    // shooter's eye forever. Refused at the encoder rather than drawn.
    if speed_mmpt == 0 {
        return Err(WireError::Range);
    }
    let mut w = begin(buf, SUB_SHOT)?;
    w.write(shooter, 32)?;
    w.write(yaw as u32, 16)?;
    w.write(pitch as u32, 8)?;
    w.write(speed_mmpt as u32, 16)?;
    w.write(drop_mmpt2 as u32, 16)?;
    Ok(w.finish())
}

/// Where an arrow stopped: a point in the entity lane's quanta and what
/// kind of surface it met (`SUB_IMPACT`).
///
/// **The window check is `write_bag`'s, verbatim and deliberately.** These
/// are the same quanta a body's position crosses in, so "is this point on
/// the island" has one answer here and there is no reason for this
/// encoder to hold a second. A stop point outside the window is the sim
/// surfacing a bug — refused, not wrapped into somebody's base.
///
/// A `surf` past [`SURF_KINDS`] is refused for the reason the field is two
/// bits wide at all: a fourth kind that truncated would decode as a live
/// one and paint bark on open ground, which is the quiet half of wire
/// drift rather than the loud half.
pub fn encode_event_impact(
    qx: i32,
    qy: i32,
    qz: i32,
    surf: u8,
    buf: &mut [u8],
) -> Result<usize, WireError> {
    if !(0..(1i64 << POS_XZ_BITS)).contains(&(qx as i64))
        || !(0..(1i64 << POS_XZ_BITS)).contains(&(qz as i64))
        || !(0..(1i64 << POS_Y_BITS)).contains(&(qy as i64 + POS_Y_BIAS as i64))
        || surf as u32 >= SURF_KINDS
    {
        return Err(WireError::Range);
    }
    let mut w = begin(buf, SUB_IMPACT)?;
    w.write(qx as u32, POS_XZ_BITS)?;
    w.write((qy + POS_Y_BIAS) as u32, POS_Y_BITS)?;
    w.write(qz as u32, POS_XZ_BITS)?;
    w.write(surf as u32, SURF_BITS)?;
    Ok(w.finish())
}

/// One swing, as the wire carries it: who. Thirty-two bits, the same width
/// `Shot` spends on its shooter, because an entity id is an entity id and a
/// narrower field here would be a second opinion about how many bodies
/// there can be.
pub fn encode_event_swing(swinger: u32, buf: &mut [u8]) -> Result<usize, WireError> {
    let mut w = begin(buf, SUB_SWING)?;
    w.write(swinger, 32)?;
    Ok(w.finish())
}

/// The attacker's hitmarker: `damage` landed on `victim`.
/// One standing backpack as the wire carries it: identity and where it
/// is, nothing else. Owner, expiry and contents stay sim-side — the
/// client needs to draw it and reach for it, and knowing whose it was or
/// how long it has left would be information the sim never promised.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WireBag {
    pub id: u32,
    pub qx: i32,
    pub qy: i32,
    pub qz: i32,
}

impl WireBag {
    /// The sim's record, narrowed to what crosses.
    pub fn of(b: &BackpackRec) -> Self {
        Self {
            id: b.id,
            qx: b.qx,
            qy: b.qy,
            qz: b.qz,
        }
    }
}

/// A bag's position must sit inside the same windows an entity's does —
/// they are the same quanta, and a bag outside the island is a server bug
/// surfacing, refused here rather than wrapped into someone's base.
fn write_bag(w: &mut BitWriter, b: &WireBag) -> Result<(), WireError> {
    if !(0..(1i64 << POS_XZ_BITS)).contains(&(b.qx as i64))
        || !(0..(1i64 << POS_XZ_BITS)).contains(&(b.qz as i64))
        || !(0..(1i64 << POS_Y_BITS)).contains(&(b.qy as i64 + POS_Y_BIAS as i64))
    {
        return Err(WireError::Range);
    }
    w.write(b.id, 32)?;
    w.write(b.qx as u32, POS_XZ_BITS)?;
    w.write((b.qy + POS_Y_BIAS) as u32, POS_Y_BITS)?;
    w.write(b.qz as u32, POS_XZ_BITS)?;
    Ok(())
}

fn read_bag(r: &mut BitReader) -> Result<WireBag, WireError> {
    Ok(WireBag {
        id: r.read(32)?,
        qx: r.read(POS_XZ_BITS)? as i32,
        qy: r.read(POS_Y_BITS)? as i32 - POS_Y_BIAS,
        qz: r.read(POS_XZ_BITS)? as i32,
    })
}

pub fn encode_event_bag_dropped(b: &WireBag, buf: &mut [u8]) -> Result<usize, WireError> {
    let mut w = begin(buf, SUB_BAG_DROPPED)?;
    write_bag(&mut w, b)?;
    Ok(w.finish())
}

/// A batch may be empty only with `reset` — the same contract as the
/// piece and deploy syncs.
pub fn encode_event_bag_sync(
    reset: bool,
    recs: &[WireBag],
    buf: &mut [u8],
) -> Result<usize, WireError> {
    if recs.len() > BAG_SYNC_BATCH || (recs.is_empty() && !reset) {
        return Err(WireError::Cap);
    }
    let mut w = begin(buf, SUB_BAG_SYNC)?;
    w.write_bit(reset)?;
    w.write(recs.len() as u32, BAG_SYNC_COUNT_BITS)?;
    for b in recs {
        write_bag(&mut w, b)?;
    }
    Ok(w.finish())
}

/// The open container's contents — see `EventMsg::ContSync`.
///
/// Three refusals, and each one is a server bug the caller wants told
/// about rather than a client fact:
///
/// 1. A kind past `CONT_MAX`.
/// 2. A close (`CONT_SELF`) that carries a handle or a slot. A close says
///    "nothing is open"; contents attached to it would be contents with no
///    container, and the client would have to invent a rule for them.
/// 3. A slot index past **that kind's** container. This is tighter than
///    the move action's bound, which checks every slot against `INV_SLOTS`
///    even for a twelve-slot box, and the asymmetry is deliberate: on the
///    action lane a tight check would make an over-wide slot a *frame*
///    error, and a frame error ends the session — the disconnect the whole
///    verb exists to avoid — so the sim answers it with a refusal event
///    instead. Here the server is the author. A box slot 17 leaving this
///    encoder is not a forged client, it is us, and the honest response is
///    `Range` into `encode_range_errors` rather than a slot the client
///    stores somewhere no reader will ever look at again.
///
/// A batch may be empty only with `reset` — the bag-sync contract, and it
/// is what makes "you opened an empty box" and "the box you opened is now
/// empty" sayable at all.
pub fn encode_event_cont_sync(
    kind: u8,
    cont: u32,
    reset: bool,
    slots: &[InvSlot],
    buf: &mut [u8],
) -> Result<usize, WireError> {
    if kind > CONT_MAX {
        return Err(WireError::Range);
    }
    if kind == CONT_SELF && (cont != 0 || !slots.is_empty() || !reset) {
        return Err(WireError::Range);
    }
    if slots.len() > CONT_SYNC_BATCH || (slots.is_empty() && !reset) {
        return Err(WireError::Cap);
    }
    let width = slots_in(kind);
    let mut w = begin(buf, SUB_CONT_SYNC)?;
    w.write(kind as u32, CONT_KIND_BITS)?;
    w.write(cont, 32)?;
    w.write_bit(reset)?;
    w.write(slots.len() as u32, CONT_COUNT_BITS)?;
    for s in slots {
        if s.slot as usize >= width {
            return Err(WireError::Range);
        }
        w.write(s.slot as u32, INV_SLOT_BITS)?;
        w.write(s.stack.item as u32, 16)?;
        w.write(s.stack.count as u32, 16)?;
        w.write(s.stack.cond as u32, 16)?;
    }
    Ok(w.finish())
}

/// The bag's exit, with its `backpack::BAG_GONE_*` reason. Bounded by the
/// sim's ledger and not by the width — the posture every refusal encoder
/// here takes (`REFUSE_M_MAX`, `REFUSE_G_MAX`, `REFUSE_C_MAX`). It checked
/// only the width until 2026-08-17, which left `why == 3` — a value the
/// ledger has no name for — encodable; nothing ever sent it (the sim
/// cannot emit one), so closing it moves no golden.
pub fn encode_event_bag_removed(id: u32, why: u8, buf: &mut [u8]) -> Result<usize, WireError> {
    if (why as u32) > BAG_GONE_MAX {
        return Err(WireError::Range);
    }
    let mut w = begin(buf, SUB_BAG_REMOVED)?;
    w.write(id, 32)?;
    w.write(why as u32, BAG_GONE_BITS)?;
    Ok(w.finish())
}

/// The move acknowledgement — see `EventMsg::Moved`. The four address
/// parts cross in the order the verb is spoken (from, then to), matching
/// `sim_core::inventory::addr`'s pack, so a transposition here reads as a
/// move in the opposite direction rather than as a different slot — which
/// is what `test_event_roles` asserts on the sim side.
pub fn encode_event_moved(
    from_kind: u8,
    from_slot: u8,
    to_kind: u8,
    to_slot: u8,
    count: u16,
    item: u16,
    buf: &mut [u8],
) -> Result<usize, WireError> {
    if from_kind > sim_core::inventory::CONT_MAX
        || to_kind > sim_core::inventory::CONT_MAX
        || from_slot as usize >= sim_core::limits::INV_SLOTS
        || to_slot as usize >= sim_core::limits::INV_SLOTS
        || count == 0
    {
        return Err(WireError::Range);
    }
    let mut w = begin(buf, SUB_MOVED)?;
    w.write(from_kind as u32, CONT_KIND_BITS)?;
    w.write(from_slot as u32, MOVE_SLOT_BITS)?;
    w.write(to_kind as u32, CONT_KIND_BITS)?;
    w.write(to_slot as u32, MOVE_SLOT_BITS)?;
    w.write(count as u32, 16)?;
    w.write(item as u32, 16)?;
    Ok(w.finish())
}

/// A refused move — see `EventMsg::MoveRefused`. A zero reason refuses for
/// `encode_event_consume_refused`'s reason: a refusal that refuses to say
/// why is the silence the whole announced-refusal posture exists to end.
///
/// The address is **not** range-checked, and that is deliberate: the
/// commonest refusal is `REFUSE_M_SLOT`, whose entire content is an
/// address the sim just rejected as out of range. An encoder that refused
/// to carry it would make the one refusal a desynced client most needs
/// unsendable. The widths clamp instead — kinds and slots are masked to
/// what the field holds, which is the same information the client needs to
/// find the drag it predicted.
pub fn encode_event_move_refused(
    reason: u8,
    from_kind: u8,
    from_slot: u8,
    to_kind: u8,
    to_slot: u8,
    buf: &mut [u8],
) -> Result<usize, WireError> {
    if reason == 0 || (reason as u32) > sim_core::inventory::REFUSE_M_MAX {
        return Err(WireError::Range);
    }
    let mut w = begin(buf, SUB_MOVE_REFUSED)?;
    w.write(reason as u32, REFUSE_M_BITS)?;
    w.write(
        (from_kind as u32) & ((1 << CONT_KIND_BITS) - 1),
        CONT_KIND_BITS,
    )?;
    w.write(
        (from_slot as u32) & ((1 << MOVE_SLOT_BITS) - 1),
        MOVE_SLOT_BITS,
    )?;
    w.write(
        (to_kind as u32) & ((1 << CONT_KIND_BITS) - 1),
        CONT_KIND_BITS,
    )?;
    w.write(
        (to_slot as u32) & ((1 << MOVE_SLOT_BITS) - 1),
        MOVE_SLOT_BITS,
    )?;
    Ok(w.finish())
}

pub fn encode_event_hit(victim: u32, damage: u16, buf: &mut [u8]) -> Result<usize, WireError> {
    let mut w = begin(buf, SUB_HIT)?;
    w.write(victim, 32)?;
    w.write(damage as u32, 16)?;
    Ok(w.finish())
}

/// The owner's health, absolute. `hp > max` is a server bug surfacing —
/// refused here rather than rendered as an over-full bar.
pub fn encode_event_health(hp: u16, max: u16, buf: &mut [u8]) -> Result<usize, WireError> {
    if hp > max {
        return Err(WireError::Range);
    }
    let mut w = begin(buf, SUB_HEALTH)?;
    w.write(hp as u32, 16)?;
    w.write(max as u32, 16)?;
    Ok(w.finish())
}

/// The meter pair, absolute. A meter above its own ceiling is refused
/// rather than clamped: it can only mean the encoder and the sim disagree
/// about the content, and a clamp would hide that behind a plausible bar.
pub fn encode_event_vitals(
    food: u16,
    water: u16,
    max_food: u16,
    max_water: u16,
    buf: &mut [u8],
) -> Result<usize, WireError> {
    if food > max_food || water > max_water {
        return Err(WireError::Range);
    }
    let mut w = begin(buf, SUB_VITALS)?;
    w.write(food as u32, 16)?;
    w.write(water as u32, 16)?;
    w.write(max_food as u32, 16)?;
    w.write(max_water as u32, 16)?;
    Ok(w.finish())
}

/// The eat acknowledgement. `slot` crosses in the inventory-slot width the
/// inv message already uses, so a slot past the sim's array is unencodable
/// rather than merely wrong.
pub fn encode_event_consumed(item: u16, slot: u8, buf: &mut [u8]) -> Result<usize, WireError> {
    if slot as usize >= INV_SLOTS {
        return Err(WireError::Range);
    }
    let mut w = begin(buf, SUB_CONSUMED)?;
    w.write(item as u32, 16)?;
    w.write(slot as u32, INV_SLOT_BITS)?;
    Ok(w.finish())
}

/// The eat refusal (`sim_core::survival::REFUSE_C_*`). Reason zero is not a
/// reason — a refusal that cannot say why is the silence this event exists
/// to replace — and anything past the sim's own ledger is a bug at this
/// end; both are refused at the encoder, the posture every other refusal
/// encoder here already takes (`REFUSE_M_MAX`, `REFUSE_G_MAX`).
pub fn encode_event_consume_refused(reason: u8, buf: &mut [u8]) -> Result<usize, WireError> {
    if reason == 0 || (reason as u32) > REFUSE_C_MAX {
        return Err(WireError::Range);
    }
    let mut w = begin(buf, SUB_CONSUME_REFUSED)?;
    w.write(reason as u32, REFUSE_C_BITS)?;
    Ok(w.finish())
}

/// The drink acknowledgement. A drink that moved no water and cost no hp
/// is not a drink — it is the refusal path, which has its own event — so
/// the encoder refuses the all-zero pair for `encode_event_consume_refused`'s
/// reason: an acknowledgement that acknowledges nothing is the silence
/// these events exist to replace.
pub fn encode_event_drank(water: u16, hp_cost: u16, buf: &mut [u8]) -> Result<usize, WireError> {
    if water == 0 && hp_cost == 0 {
        return Err(WireError::Range);
    }
    let mut w = begin(buf, SUB_DRANK)?;
    w.write(water as u32, 16)?;
    w.write(hp_cost as u32, 16)?;
    Ok(w.finish())
}

/// A death, broadcast — with what the death screen and the kill feed both
/// need to say a sentence rather than a name. `cause` is a
/// `sim_core::world::DEATH_BY_*` code; the width is the range check, so a
/// forged one cannot decode.
pub fn encode_event_death(
    victim: u32,
    killer: u32,
    cause: u8,
    item: u16,
    range_cm: u16,
    buf: &mut [u8],
) -> Result<usize, WireError> {
    if cause > DEATH_CAUSE_MAX {
        return Err(WireError::Range);
    }
    let mut w = begin(buf, SUB_DEATH)?;
    w.write(victim, 32)?;
    w.write(killer, 32)?;
    w.write(cause as u32, DEATH_CAUSE_BITS)?;
    w.write(item as u32, 16)?;
    w.write(range_cm as u32, 16)?;
    Ok(w.finish())
}

/// A body woke up, own-fact — see `EventMsg::Respawn`.
pub fn encode_event_respawn(on_bag: bool, buf: &mut [u8]) -> Result<usize, WireError> {
    let mut w = begin(buf, SUB_RESPAWN)?;
    w.write_bit(on_bag)?;
    Ok(w.finish())
}

/// Relay one chat line to one recipient. The text is already sanitized
/// (`ChatText` has no other constructor), so this cannot put a line on
/// the wire the C→S decoder would have refused.
pub fn encode_event_chat(
    from: u32,
    global: bool,
    text: &ChatText,
    buf: &mut [u8],
) -> Result<usize, WireError> {
    let mut w = begin(buf, SUB_CHAT)?;
    w.write(from, 32)?;
    w.write_bit(global)?;
    write_text(&mut w, text)?;
    Ok(w.finish())
}

/// Total decode of one event-lane message: arbitrary bytes in, `Ok` or a
/// `WireError` out, never a panic — same contract as the datagrams.
pub fn decode_event(buf: &[u8]) -> Result<EventMsg, WireError> {
    let mut r = BitReader::new(buf);
    if r.read(KIND_BITS)? != KIND_EVENT {
        return Err(WireError::Malformed);
    }
    let msg = match r.read(SUB_BITS)? {
        SUB_GATHER => EventMsg::Gather {
            item: r.read(16)? as u16,
            added: r.read(16)? as u16,
        },
        SUB_GATHER_REFUSED => {
            let item = r.read(16)? as u16;
            let reason = r.read(REFUSE_G_BITS)? as u8;
            // Zero is refused (`SUB_CONSUME_REFUSED`'s posture exactly);
            // the upper end is deliberately NOT bounded here, also that
            // refusal's posture — a shard newer than this client can
            // legitimately send a reason it has no word for, and the
            // client's table prints it as `code N` instead of dropping
            // the frame (the domain table's stated forgery-slack call).
            if reason == 0 {
                return Err(WireError::Malformed);
            }
            EventMsg::GatherRefused { item, reason }
        }
        SUB_INV => {
            let count = r.read(INV_COUNT_BITS)? as usize;
            if count == 0 || count > INV_SLOTS {
                return Err(WireError::Malformed);
            }
            let mut slots = [InvSlot::default(); INV_SLOTS];
            for s in slots.iter_mut().take(count) {
                let slot = r.read(INV_SLOT_BITS)? as u8;
                if slot as usize >= INV_SLOTS {
                    return Err(WireError::Malformed);
                }
                *s = InvSlot {
                    slot,
                    stack: ItemStack {
                        item: r.read(16)? as u16,
                        count: r.read(16)? as u16,
                        cond: r.read(16)? as u16,
                    },
                };
            }
            EventMsg::Inv {
                slots,
                count: count as u8,
            }
        }
        sub @ (SUB_SLOT_HARVESTED | SUB_SLOT_RESPAWNED) => {
            let cx = r.read(16)? as u16;
            let cz = r.read(16)? as u16;
            if sub == SUB_SLOT_HARVESTED {
                EventMsg::SlotHarvested { cx, cz }
            } else {
                EventMsg::SlotRespawned { cx, cz }
            }
        }
        SUB_SLOT_SYNC => {
            let reset = r.read_bit()?;
            let count = r.read(SYNC_COUNT_BITS)? as usize;
            if count > SLOT_SYNC_BATCH || (count == 0 && !reset) {
                return Err(WireError::Malformed);
            }
            let mut cells = [(0u16, 0u16); SLOT_SYNC_BATCH];
            for c in cells.iter_mut().take(count) {
                *c = (r.read(16)? as u16, r.read(16)? as u16);
            }
            EventMsg::SlotSync {
                reset,
                cells,
                count: count as u8,
            }
        }
        SUB_CATALOG => {
            let total = r.read(CATALOG_TOTAL_BITS)? as usize;
            let first = r.read(CATALOG_TOTAL_BITS)? as usize;
            let count = r.read(CATALOG_COUNT_BITS)? as usize;
            if total > MAX_ITEM_DEFS || count == 0 || count > CATALOG_BATCH || first + count > total
            {
                return Err(WireError::Malformed);
            }
            let mut names = [[0u8; MAX_ITEM_NAME_BYTES]; CATALOG_BATCH];
            let mut lens = [0u8; CATALOG_BATCH];
            let mut cond_max = [0u16; CATALOG_BATCH];
            for i in 0..count {
                let len = r.read(NAME_LEN_BITS)? as usize;
                if len == 0 || len > MAX_ITEM_NAME_BYTES {
                    return Err(WireError::Malformed);
                }
                for b in names[i].iter_mut().take(len) {
                    *b = r.read(8)? as u8;
                }
                lens[i] = len as u8;
                cond_max[i] = r.read(16)? as u16;
            }
            EventMsg::Catalog {
                total: total as u8,
                first: first as u8,
                count: count as u8,
                names,
                lens,
                cond_max,
            }
        }
        SUB_WEAK_MARK => EventMsg::WeakMark {
            cx: r.read(16)? as u16,
            cz: r.read(16)? as u16,
            mark8: r.read(8)? as u8,
            weak_hit: r.read_bit()?,
        },
        SUB_CRAFT_Q => {
            let count = r.read(CRAFT_Q_COUNT_BITS)? as usize;
            if count > CRAFT_QUEUE {
                return Err(WireError::Malformed);
            }
            let mut jobs = [(0u8, 0u8); CRAFT_QUEUE];
            for j in jobs.iter_mut().take(count) {
                let recipe = r.read(8)? as u8;
                let remaining = r.read(8)? as u8;
                if recipe as usize >= MAX_RECIPES || remaining == 0 {
                    return Err(WireError::Malformed);
                }
                *j = (recipe, remaining);
            }
            EventMsg::CraftQ {
                jobs,
                count: count as u8,
                eta_ticks: r.read(16)? as u16,
            }
        }
        SUB_CRAFT_DONE => EventMsg::CraftDone {
            item: r.read(16)? as u16,
            added: r.read(16)? as u16,
        },
        SUB_CRAFT_REFUSED => EventMsg::CraftRefused {
            reason: r.read(8)? as u8,
        },
        SUB_RECIPES => {
            let total = r.read(RECIPE_TOTAL_BITS)? as usize;
            let first = r.read(RECIPE_TOTAL_BITS)? as usize;
            let count = r.read(RECIPE_COUNT_BITS)? as usize;
            if total > MAX_RECIPES || count == 0 || count > RECIPE_BATCH || first + count > total {
                return Err(WireError::Malformed);
            }
            let mut rows = [RecipeDef::INERT; RECIPE_BATCH];
            for row in rows.iter_mut().take(count) {
                let output = r.read(16)? as u16;
                let out_count = r.read(8)? as u16;
                let ticks = r.read(RECIPE_TICKS_BITS)?;
                let station = r.read(STATION_BITS)? as u8;
                let blueprint = r.read_bit()?;
                let n_inputs = r.read(N_INPUTS_BITS)? as u8;
                if out_count == 0
                    || ticks == 0
                    || station > STATION_MAX
                    || n_inputs == 0
                    || n_inputs as usize > MAX_RECIPE_INPUTS
                {
                    return Err(WireError::Malformed);
                }
                let mut inputs = [(0u16, 0u16); MAX_RECIPE_INPUTS];
                for input in inputs.iter_mut().take(n_inputs as usize) {
                    *input = (r.read(16)? as u16, r.read(16)? as u16);
                }
                *row = RecipeDef {
                    output,
                    out_count,
                    ticks,
                    station,
                    blueprint,
                    n_inputs,
                    inputs,
                };
            }
            EventMsg::Recipes {
                total: total as u8,
                first: first as u8,
                count: count as u8,
                rows,
            }
        }
        // The research lane's three S→C arms. They did not exist from v32
        // through v37: the encoders landed with research v0 and the
        // decoder's `_ => Malformed` ate every one of them, so a client
        // never saw a research toast, a refusal sentence or a `Known`
        // restate — and nothing said so, because no golden pinned these
        // bytes and the role gate checks payloads at the sim, not the
        // codec. Found by the v38 fixtures the moment they existed, which
        // is the argument for pinning a lane in the same commit that
        // opens it.
        SUB_RESEARCH => {
            let recipe = r.read(16)? as u16;
            let cost = r.read(16)? as u16;
            if recipe as usize >= MAX_RECIPES {
                return Err(WireError::Malformed);
            }
            EventMsg::Research { recipe, cost }
        }
        SUB_RESEARCH_REFUSED => {
            let reason = r.read(RESEARCH_REFUSE_BITS)? as u8;
            if reason as u32 > sim_core::research::REFUSE_R_MAX {
                return Err(WireError::Malformed);
            }
            EventMsg::ResearchRefused { reason }
        }
        SUB_KNOWN => {
            let lo = r.read(32)?;
            let hi = r.read(32)?;
            EventMsg::Known {
                mask: (hi as u64) << 32 | lo as u64,
            }
        }
        SUB_BAGS => {
            // `BAGS_COUNT_BITS` holds 0..=15 and `BAG_CAP` is 8, so the
            // four values above the cap are forgeable and refuse here —
            // the hotbar selector's posture, and the reason a count is
            // never trusted straight into an index.
            let count = r.read(BAGS_COUNT_BITS)? as usize;
            if count > BAG_CAP {
                return Err(WireError::Malformed);
            }
            let mut bags = [BagAnchor::default(); BAG_CAP];
            for b in bags.iter_mut().take(count) {
                *b = BagAnchor {
                    cx: r.read(BUILD_CELL_BITS)? as u16,
                    cz: r.read(BUILD_CELL_BITS)? as u16,
                    level: r.read(BUILD_LEVEL_BITS)? as u8,
                    ready: r.read_bit()?,
                };
            }
            EventMsg::Bags {
                bags,
                count: count as u8,
            }
        }
        SUB_RESEARCH_ROWS => {
            let total = r.read(RESEARCH_TOTAL_BITS)? as usize;
            let first = r.read(RESEARCH_TOTAL_BITS)? as usize;
            let count = r.read(RESEARCH_COUNT_BITS)? as usize;
            if total > MAX_RESEARCH_ROWS
                || count == 0
                || count > RESEARCH_BATCH
                || first + count > total
            {
                return Err(WireError::Malformed);
            }
            let coin = r.read(16)? as u16;
            let mut rows = [ResearchRow::INERT; RESEARCH_BATCH];
            for row in rows.iter_mut().take(count) {
                let item = r.read(16)? as u16;
                let recipe = r.read(8)? as u16;
                let cost = r.read(16)? as u16;
                let req = r.read(8)? as u16;
                // 0xFF is the wire's NO_RECIPE; anything else must name a
                // live recipe index or the row is forged.
                let requires = if req == 0xFF { NO_RECIPE } else { req };
                if recipe as usize >= MAX_RECIPES
                    || (requires != NO_RECIPE && requires as usize >= MAX_RECIPES)
                {
                    return Err(WireError::Malformed);
                }
                *row = ResearchRow {
                    item,
                    recipe,
                    cost,
                    requires,
                };
            }
            EventMsg::ResearchRows {
                total: total as u8,
                first: first as u8,
                count: count as u8,
                coin,
                rows,
            }
        }
        SUB_PIECE_PLACED => EventMsg::PiecePlaced {
            rec: read_piece_rec(&mut r)?,
        },
        SUB_PIECE_SYNC => {
            let reset = r.read_bit()?;
            let count = r.read(PIECE_SYNC_COUNT_BITS)? as usize;
            if count > PIECE_SYNC_BATCH || (count == 0 && !reset) {
                return Err(WireError::Malformed);
            }
            let mut recs = [PieceRec::default(); PIECE_SYNC_BATCH];
            for rec in recs.iter_mut().take(count) {
                *rec = read_piece_rec(&mut r)?;
            }
            EventMsg::PieceSync {
                reset,
                recs,
                count: count as u8,
            }
        }
        SUB_BUILD_REFUSED => EventMsg::BuildRefused {
            reason: r.read(REFUSE_B_BITS)? as u8,
        },
        SUB_PIECE_REPAIRED => {
            let deploy = r.read_bit()?;
            let cx = r.read(BUILD_CELL_BITS)? as u16;
            let cz = r.read(BUILD_CELL_BITS)? as u16;
            let level = r.read(BUILD_LEVEL_BITS)? as u8;
            let loc = r.read(BUILD_LOC_BITS)? as u8;
            let row = r.read(if deploy {
                DEPLOY_ROW_BITS
            } else {
                PIECE_ROW_BITS
            })? as u8;
            let healed = r.read(16)? as u16;
            let hp = r.read(16)? as u16;
            // The encoder's own refusals, restated: the loc is bounded by
            // the STORE the bit named (v40), and a zero heal, a zero
            // ceiling, or a heal past the ceiling are all forgeable and
            // all would corrupt an hp mirror. `row` needs no bound — the
            // width the bit selected is exactly its domain, which is why
            // the field is read at that width rather than masked after.
            if loc > loc_max(deploy) || healed == 0 || hp == 0 || healed > hp {
                return Err(WireError::Malformed);
            }
            EventMsg::PieceRepaired {
                deploy,
                cx,
                cz,
                level,
                loc,
                row,
                healed,
                hp,
            }
        }
        SUB_CHARGE_PLACED => {
            let deploy = r.read_bit()?;
            let cx = r.read(BUILD_CELL_BITS)? as u16;
            let cz = r.read(BUILD_CELL_BITS)? as u16;
            let level = r.read(BUILD_LEVEL_BITS)? as u8;
            let loc = r.read(BUILD_LOC_BITS)? as u8;
            let row = r.read(if deploy {
                DEPLOY_ROW_BITS
            } else {
                PIECE_ROW_BITS
            })? as u8;
            let fuse = r.read(16)? as u16;
            // The encoder's refusals, restated — `PieceRepaired`'s arm
            // exactly, minus the pair it does not carry. `row` needs no
            // bound: the width the store bit selected is its domain.
            if loc > loc_max(deploy) || fuse == 0 {
                return Err(WireError::Malformed);
            }
            EventMsg::ChargePlaced {
                deploy,
                cx,
                cz,
                level,
                loc,
                row,
                fuse,
            }
        }
        SUB_PIECE_DEFS => {
            let total = r.read(PIECE_DEFS_TOTAL_BITS)? as usize;
            let first = r.read(PIECE_DEFS_TOTAL_BITS)? as usize;
            let count = r.read(PIECE_DEFS_COUNT_BITS)? as usize;
            if total > MAX_PIECE_DEFS
                || count == 0
                || count > PIECE_DEFS_BATCH
                || first + count > total
            {
                return Err(WireError::Malformed);
            }
            let mut rows = [PieceDef::INERT; PIECE_DEFS_BATCH];
            for row in rows.iter_mut().take(count) {
                let shape = r.read(SHAPE_BITS)? as u8;
                let material = r.read(MATERIAL_BITS)? as u8;
                let hp = r.read(16)? as u16;
                let n_costs = r.read(N_COSTS_BITS)? as u8;
                if shape > SHAPE_TRI_ROOF
                    || material > MAT_METAL
                    || hp == 0
                    || n_costs == 0
                    || n_costs as usize > MAX_PIECE_COSTS
                {
                    return Err(WireError::Malformed);
                }
                let mut costs = [(0u16, 0u16); MAX_PIECE_COSTS];
                for cost in costs.iter_mut().take(n_costs as usize) {
                    *cost = (r.read(16)? as u16, r.read(16)? as u16);
                }
                *row = PieceDef {
                    shape,
                    material,
                    hp,
                    n_costs,
                    costs,
                };
            }
            EventMsg::PieceDefs {
                total: total as u8,
                first: first as u8,
                count: count as u8,
                rows,
            }
        }
        SUB_DEPLOY_PLACED => EventMsg::DeployPlaced {
            rec: read_deploy_rec(&mut r)?,
        },
        SUB_DEPLOY_SYNC => {
            let reset = r.read_bit()?;
            let count = r.read(DEPLOY_SYNC_COUNT_BITS)? as usize;
            if count > DEPLOY_SYNC_BATCH || (count == 0 && !reset) {
                return Err(WireError::Malformed);
            }
            let mut recs = [DeployRec::default(); DEPLOY_SYNC_BATCH];
            for rec in recs.iter_mut().take(count) {
                *rec = read_deploy_rec(&mut r)?;
            }
            EventMsg::DeploySync {
                reset,
                recs,
                count: count as u8,
            }
        }
        SUB_DEPLOY_REFUSED => EventMsg::DeployRefused {
            reason: r.read(8)? as u8,
        },
        SUB_DEPLOY_DEFS => {
            let total = r.read(DEPLOY_DEFS_TOTAL_BITS)? as usize;
            let first = r.read(DEPLOY_DEFS_TOTAL_BITS)? as usize;
            let count = r.read(DEPLOY_DEFS_COUNT_BITS)? as usize;
            if total > MAX_DEPLOY_DEFS
                || count == 0
                || count > DEPLOY_DEFS_BATCH
                || first + count > total
            {
                return Err(WireError::Malformed);
            }
            let mut rows = [DeployDef::INERT; DEPLOY_DEFS_BATCH];
            for row in rows.iter_mut().take(count) {
                let arch = r.read(ARCH_BITS)? as u8;
                let placement = r.read(PLACEMENT_BITS)? as u8;
                let hp = r.read(16)? as u16;
                let item = r.read(16)? as u16;
                let n_costs = r.read(DEPLOY_COSTS_BITS)? as u8;
                if arch > ARCH_WORKBENCH3
                    || placement > PLACE_DOOR
                    || hp == 0
                    || n_costs as usize > MAX_DEPLOY_COSTS
                {
                    return Err(WireError::Malformed);
                }
                let mut costs = [(0u16, 0u16); MAX_DEPLOY_COSTS];
                for cost in costs.iter_mut().take(n_costs as usize) {
                    *cost = (r.read(16)? as u16, r.read(16)? as u16);
                }
                *row = DeployDef {
                    arch,
                    placement,
                    hp,
                    item,
                    n_costs,
                    costs,
                };
            }
            EventMsg::DeployDefs {
                total: total as u8,
                first: first as u8,
                count: count as u8,
                rows,
            }
        }
        SUB_STRUCT_HIT => {
            let deploy = r.read_bit()?;
            let cx = r.read(BUILD_CELL_BITS)? as u16;
            let cz = r.read(BUILD_CELL_BITS)? as u16;
            let level = r.read(BUILD_LEVEL_BITS)? as u8;
            let loc = r.read(BUILD_LOC_BITS)? as u8;
            let _row = r.read(if deploy {
                DEPLOY_ROW_BITS
            } else {
                PIECE_ROW_BITS
            })?;
            let damage = r.read(16)? as u16;
            let left = r.read(16)? as u16;
            if loc > loc_max(deploy) || left == 0 {
                return Err(WireError::Malformed);
            }
            EventMsg::StructHit {
                deploy,
                cx,
                cz,
                level,
                loc,
                damage,
                left,
            }
        }
        sub @ (SUB_PIECE_REMOVED | SUB_DEPLOY_REMOVED) => {
            let cx = r.read(BUILD_CELL_BITS)? as u16;
            let cz = r.read(BUILD_CELL_BITS)? as u16;
            let level = r.read(BUILD_LEVEL_BITS)? as u8;
            let loc = r.read(BUILD_LOC_BITS)? as u8;
            // The subtype IS the store bit here, and the store bounds the
            // loc (v40).
            if loc > loc_max(sub == SUB_DEPLOY_REMOVED) {
                return Err(WireError::Malformed);
            }
            if sub == SUB_PIECE_REMOVED {
                EventMsg::PieceRemoved { cx, cz, level, loc }
            } else {
                EventMsg::DeployRemoved { cx, cz, level, loc }
            }
        }
        SUB_STOCK => {
            let cx = r.read(BUILD_CELL_BITS)? as u16;
            let cz = r.read(BUILD_CELL_BITS)? as u16;
            let level = r.read(BUILD_LEVEL_BITS)? as u8;
            let count = r.read(STOCK_COUNT_BITS)? as usize;
            if count > HEARTH_STOCK_ROWS {
                return Err(WireError::Malformed);
            }
            let mut rows = [(0u16, 0u32); HEARTH_STOCK_ROWS];
            for row in rows.iter_mut().take(count) {
                *row = (r.read(16)? as u16, r.read(32)?);
            }
            EventMsg::Stock {
                cx,
                cz,
                level,
                rows,
                count: count as u8,
            }
        }
        // Address + state, every width exact — nothing to forge.
        SUB_DOOR => {
            let m = EventMsg::Door {
                cx: r.read(BUILD_CELL_BITS)? as u16,
                cz: r.read(BUILD_CELL_BITS)? as u16,
                level: r.read(BUILD_LEVEL_BITS)? as u8,
                loc: r.read(BUILD_LOC_BITS)? as u8,
                open: r.read_bit()?,
                locked: r.read_bit()?,
                has_lock: r.read_bit()?,
            };
            // A door hangs on a straight edge and nowhere else (v40).
            if matches!(m, EventMsg::Door { loc, .. } if loc > loc_max(true)) {
                return Err(WireError::Malformed);
            }
            m
        }
        SUB_SHOT => {
            let m = EventMsg::Shot {
                shooter: r.read(32)?,
                yaw: r.read(16)? as u16,
                pitch: r.read(8)? as u8,
                speed_mmpt: r.read(16)? as u16,
                drop_mmpt2: r.read(16)? as u16,
            };
            // The encoder's refusal, mirrored: a zero-speed tracer is a
            // value no honest sender produces, so it is malformed rather
            // than drawn. Same posture as the address ranges above.
            if matches!(m, EventMsg::Shot { speed_mmpt: 0, .. }) {
                return Err(WireError::Malformed);
            }
            m
        }
        SUB_SWING => EventMsg::Swing {
            swinger: r.read(32)?,
        },
        SUB_IMPACT => {
            let qx = r.read(POS_XZ_BITS)? as i32;
            let qy = r.read(POS_Y_BITS)? as i32 - POS_Y_BIAS;
            let qz = r.read(POS_XZ_BITS)? as i32;
            let surf = r.read(SURF_BITS)?;
            // The encoder's refusal, mirrored — `SUB_SHOT`'s posture one
            // arm up. Two bits hold four values and the sim makes three,
            // so the fourth is a sender this build does not understand
            // and the honest answer is malformed rather than a guess at
            // which mark it meant.
            if surf >= SURF_KINDS {
                return Err(WireError::Malformed);
            }
            EventMsg::Impact {
                qx,
                qy,
                qz,
                surf: surf as u8,
            }
        }
        SUB_OVEN => EventMsg::Oven {
            cx: r.read(BUILD_CELL_BITS)? as u16,
            cz: r.read(BUILD_CELL_BITS)? as u16,
            level: r.read(BUILD_LEVEL_BITS)? as u8,
            lit: r.read_bit()?,
            by: r.read(32)?,
        },
        SUB_KNOCK => {
            let m = EventMsg::Knock {
                cx: r.read(BUILD_CELL_BITS)? as u16,
                cz: r.read(BUILD_CELL_BITS)? as u16,
                level: r.read(BUILD_LEVEL_BITS)? as u8,
                loc: r.read(BUILD_LOC_BITS)? as u8,
                by: r.read(32)?,
            };
            // A knock lands on a door's address — the deploy bound (v40).
            if matches!(m, EventMsg::Knock { loc, .. } if loc > loc_max(true)) {
                return Err(WireError::Malformed);
            }
            m
        }
        // The grant is two bits holding three values, so the fourth is
        // forgeable and refused rather than clamped — a client told it
        // holds rights the sim never granted would draw an open door it
        // cannot open.
        SUB_AUTH => {
            let cx = r.read(BUILD_CELL_BITS)? as u16;
            let cz = r.read(BUILD_CELL_BITS)? as u16;
            let level = r.read(BUILD_LEVEL_BITS)? as u8;
            let loc = r.read(BUILD_LOC_BITS)? as u8;
            let grant = r.read(LOCK_GRANT_BITS)? as u8;
            if grant > sim_core::lock::GRANT_FULL {
                return Err(WireError::Malformed);
            }
            EventMsg::Auth {
                cx,
                cz,
                level,
                loc,
                grant,
            }
        }
        // The relay is held to the sender's own rule: `read_text`
        // sanitizes or refuses, so a client never renders a line the
        // server would not have accepted.
        SUB_CHAT => {
            let from = r.read(32)?;
            let global = r.read_bit()?;
            EventMsg::Chat {
                from,
                global,
                text: read_text(&mut r)?,
            }
        }
        SUB_HIT => EventMsg::Hit {
            victim: r.read(32)?,
            damage: r.read(16)? as u16,
        },
        SUB_HEALTH => {
            let hp = r.read(16)? as u16;
            let max = r.read(16)? as u16;
            if hp > max {
                return Err(WireError::Malformed);
            }
            EventMsg::Health { hp, max }
        }
        SUB_VITALS => {
            let food = r.read(16)? as u16;
            let water = r.read(16)? as u16;
            let max_food = r.read(16)? as u16;
            let max_water = r.read(16)? as u16;
            if food > max_food || water > max_water {
                return Err(WireError::Malformed);
            }
            EventMsg::Vitals {
                food,
                water,
                max_food,
                max_water,
            }
        }
        SUB_CONSUMED => {
            let item = r.read(16)? as u16;
            let slot = r.read(INV_SLOT_BITS)? as u8;
            if slot as usize >= INV_SLOTS {
                return Err(WireError::Malformed);
            }
            EventMsg::Consumed { item, slot }
        }
        // Zero is "no reason" and 4..=15 name nothing in the sim's ledger
        // — both are refused, not passed through. Narrowed 2026-08-17 with
        // **no version turn**: see the narrowing rule at `PROTO_VER`
        // (lib.rs) — the encoder never let these values out, so no
        // compliant peer's bytes change meaning.
        SUB_CONSUME_REFUSED => {
            let reason = r.read(REFUSE_C_BITS)? as u8;
            if reason == 0 || (reason as u32) > REFUSE_C_MAX {
                return Err(WireError::Malformed);
            }
            EventMsg::ConsumeRefused { reason }
        }
        SUB_DRANK => {
            let water = r.read(16)? as u16;
            let hp_cost = r.read(16)? as u16;
            if water == 0 && hp_cost == 0 {
                return Err(WireError::Malformed);
            }
            EventMsg::Drank { water, hp_cost }
        }
        SUB_DEATH => {
            let victim = r.read(32)?;
            let killer = r.read(32)?;
            let cause = r.read(DEATH_CAUSE_BITS)? as u8;
            if cause > DEATH_CAUSE_MAX {
                return Err(WireError::Malformed);
            }
            EventMsg::Death {
                victim,
                killer,
                cause,
                item: r.read(16)? as u16,
                range_cm: r.read(16)? as u16,
            }
        }
        SUB_MOVED => {
            let from_kind = r.read(CONT_KIND_BITS)? as u8;
            let from_slot = r.read(MOVE_SLOT_BITS)? as u8;
            let to_kind = r.read(CONT_KIND_BITS)? as u8;
            let to_slot = r.read(MOVE_SLOT_BITS)? as u8;
            let count = r.read(16)? as u16;
            let item = r.read(16)? as u16;
            if from_kind > sim_core::inventory::CONT_MAX
                || to_kind > sim_core::inventory::CONT_MAX
                || from_slot as usize >= sim_core::limits::INV_SLOTS
                || to_slot as usize >= sim_core::limits::INV_SLOTS
                || count == 0
            {
                return Err(WireError::Malformed);
            }
            EventMsg::Moved {
                from_kind,
                from_slot,
                to_kind,
                to_slot,
                count,
                item,
            }
        }
        SUB_MOVE_REFUSED => {
            let reason = r.read(REFUSE_M_BITS)? as u8;
            // The reason is checked and the address is not — see
            // `encode_event_move_refused`: a `REFUSE_M_SLOT` refusal
            // exists precisely to carry an address the sim rejected.
            if reason == 0 || (reason as u32) > sim_core::inventory::REFUSE_M_MAX {
                return Err(WireError::Malformed);
            }
            EventMsg::MoveRefused {
                reason,
                from_kind: r.read(CONT_KIND_BITS)? as u8,
                from_slot: r.read(MOVE_SLOT_BITS)? as u8,
                to_kind: r.read(CONT_KIND_BITS)? as u8,
                to_slot: r.read(MOVE_SLOT_BITS)? as u8,
            }
        }
        SUB_RESPAWN => EventMsg::Respawn {
            on_bag: r.read_bit()?,
        },
        SUB_BAG_DROPPED => {
            let b = read_bag(&mut r)?;
            EventMsg::BagDropped {
                id: b.id,
                qx: b.qx,
                qy: b.qy,
                qz: b.qz,
            }
        }
        SUB_BAG_SYNC => {
            let reset = r.read_bit()?;
            let count = r.read(BAG_SYNC_COUNT_BITS)? as usize;
            if count > BAG_SYNC_BATCH || (count == 0 && !reset) {
                return Err(WireError::Malformed);
            }
            let mut recs = [WireBag::default(); BAG_SYNC_BATCH];
            for rec in recs.iter_mut().take(count) {
                *rec = read_bag(&mut r)?;
            }
            EventMsg::BagSync {
                reset,
                recs,
                count: count as u8,
            }
        }
        SUB_CONT_SYNC => {
            let kind = r.read(CONT_KIND_BITS)? as u8;
            let cont = r.read(32)?;
            let reset = r.read_bit()?;
            let count = r.read(CONT_COUNT_BITS)? as usize;
            // Every refusal the encoder makes, made again — this decoder
            // is the client's, and a server it does not trust is exactly
            // the case a codec is written total for.
            if kind > CONT_MAX
                || count > CONT_SYNC_BATCH
                || (count == 0 && !reset)
                || (kind == CONT_SELF && (cont != 0 || count != 0 || !reset))
            {
                return Err(WireError::Malformed);
            }
            let width = slots_in(kind);
            let mut slots = [InvSlot::default(); CONT_SYNC_BATCH];
            for s in slots.iter_mut().take(count) {
                let slot = r.read(INV_SLOT_BITS)? as u8;
                if slot as usize >= width {
                    return Err(WireError::Malformed);
                }
                *s = InvSlot {
                    slot,
                    stack: ItemStack {
                        item: r.read(16)? as u16,
                        count: r.read(16)? as u16,
                        cond: r.read(16)? as u16,
                    },
                };
            }
            EventMsg::ContSync {
                kind,
                cont,
                reset,
                slots,
                count: count as u8,
            }
        }
        // The width's fourth value names no `BAG_GONE_*` and is refused —
        // `SUB_CONSUME_REFUSED`'s narrowing, same date, same no-bump
        // reasoning (the narrowing rule at `PROTO_VER`, lib.rs). Until
        // then a forged `why == 3` decoded intact and reached the HUD as
        // a value no rule owns.
        SUB_BAG_REMOVED => {
            let id = r.read(32)?;
            let why = r.read(BAG_GONE_BITS)? as u8;
            if (why as u32) > BAG_GONE_MAX {
                return Err(WireError::Malformed);
            }
            EventMsg::BagRemoved { id, why }
        }
        _ => return Err(WireError::Malformed),
    };
    expect_zero_padding(&mut r)?;
    Ok(msg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_core::inventory::{CONT_BAG, CONT_BOX};
    use sim_core::limits::BOX_SLOTS;

    #[test]
    fn gather_and_slot_change_round_trip() {
        let mut buf = [0u8; MAX_EVENT_MSG_BYTES];
        let len = encode_event_gather(7, 13, &mut buf).unwrap();
        assert_eq!(
            decode_event(&buf[..len]).unwrap(),
            EventMsg::Gather { item: 7, added: 13 }
        );
        let len = encode_event_slot_change(true, 130, 77, &mut buf).unwrap();
        assert_eq!(
            decode_event(&buf[..len]).unwrap(),
            EventMsg::SlotHarvested { cx: 130, cz: 77 }
        );
        let len = encode_event_slot_change(false, 130, 77, &mut buf).unwrap();
        assert_eq!(
            decode_event(&buf[..len]).unwrap(),
            EventMsg::SlotRespawned { cx: 130, cz: 77 }
        );
    }

    /// **The container sync carries condition** (item durability v0, gate
    /// 8) — on both lanes that carry slots, proven by roundtrip equality
    /// against the INPUT and deliberately not against a fixture: the
    /// golden was regenerated from this same encoder, so zeroing the
    /// `cond` write would regenerate a matching golden and stay green
    /// there, while this check reads 0 where it wrote 4 660 and goes red.
    #[test]
    fn a_worn_slot_crosses_both_lanes_with_its_condition() {
        let mut buf = [0u8; MAX_EVENT_MSG_BYTES];
        let worn = InvSlot {
            slot: 3,
            stack: ItemStack {
                item: 7,
                count: 1,
                cond: 0x1234,
            },
        };
        let len = encode_event_inv(&[worn], &mut buf).unwrap();
        match decode_event(&buf[..len]).unwrap() {
            EventMsg::Inv { slots, .. } => assert_eq!(
                slots[0].stack.cond, 0x1234,
                "SUB_INV dropped the condition — a worn tool arrives whole"
            ),
            other => panic!("wrong variant: {other:?}"),
        }
        let len = encode_event_cont_sync(CONT_BAG, 5, false, &[worn], &mut buf).unwrap();
        match decode_event(&buf[..len]).unwrap() {
            EventMsg::ContSync { slots, .. } => assert_eq!(
                slots[0].stack.cond, 0x1234,
                "SUB_CONT_SYNC dropped the condition — a worn tool in a \
                 container arrives whole"
            ),
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn inv_round_trips_and_refuses_bad_shapes() {
        let mut buf = [0u8; MAX_EVENT_MSG_BYTES];
        let mut slots = [InvSlot::default(); INV_SLOTS];
        for (i, s) in slots.iter_mut().enumerate() {
            *s = InvSlot {
                slot: i as u8,
                stack: ItemStack {
                    item: i as u16,
                    count: 100 + i as u16,
                    cond: 200 + i as u16,
                },
            };
        }
        let len = encode_event_inv(&slots, &mut buf).unwrap();
        match decode_event(&buf[..len]).unwrap() {
            EventMsg::Inv { slots: got, count } => {
                assert_eq!(count as usize, INV_SLOTS);
                assert_eq!(got, slots);
            }
            other => panic!("wrong variant: {other:?}"),
        }
        assert_eq!(encode_event_inv(&[], &mut buf), Err(WireError::Cap));
        let bad = [InvSlot {
            slot: INV_SLOTS as u8,
            stack: ItemStack::default(),
        }];
        assert_eq!(encode_event_inv(&bad, &mut buf), Err(WireError::Range));
    }

    #[test]
    fn slot_sync_full_batch_fits_the_cap() {
        let mut buf = [0u8; MAX_EVENT_MSG_BYTES];
        let cells: [(u16, u16); SLOT_SYNC_BATCH] =
            core::array::from_fn(|i| (i as u16, (i * 3) as u16));
        let len = encode_event_slot_sync(true, &cells, &mut buf).unwrap();
        assert!(len <= MAX_EVENT_MSG_BYTES);
        match decode_event(&buf[..len]).unwrap() {
            EventMsg::SlotSync {
                reset,
                cells: got,
                count,
            } => {
                assert!(reset);
                assert_eq!(count as usize, SLOT_SYNC_BATCH);
                assert_eq!(got, cells);
            }
            other => panic!("wrong variant: {other:?}"),
        }
        // Empty is only a message when it resets.
        assert!(encode_event_slot_sync(true, &[], &mut buf).is_ok());
        assert_eq!(
            encode_event_slot_sync(false, &[], &mut buf),
            Err(WireError::Cap)
        );
    }

    #[test]
    fn catalog_batches_walk_the_table_within_cap() {
        let mut cat = ItemCatalog::EMPTY;
        cat.count = 11;
        for i in 0..11usize {
            // Worst-width names so the cap check is honest; ceilings mix
            // zero (no condition) with the full u16 corner.
            let name = [b'a' + (i as u8 % 26); MAX_ITEM_NAME_BYTES];
            let cond_max = if i % 2 == 0 { 0 } else { u16::MAX - i as u16 };
            cat.set(i, &name, cond_max).unwrap();
        }
        let mut buf = [0u8; MAX_EVENT_MSG_BYTES];
        let (len, took) = encode_event_catalog(&cat, 0, &mut buf).unwrap();
        assert!(len <= MAX_EVENT_MSG_BYTES);
        assert_eq!(took, CATALOG_BATCH);
        match decode_event(&buf[..len]).unwrap() {
            EventMsg::Catalog {
                total,
                first,
                count,
                names,
                lens,
                cond_max,
            } => {
                assert_eq!((total, first, count), (11, 0, CATALOG_BATCH as u8));
                assert_eq!(&names[0][..lens[0] as usize], cat.name(0));
                for (i, &cm) in cond_max.iter().enumerate().take(CATALOG_BATCH) {
                    assert_eq!(cm, cat.cond_max(i), "ceiling column drifted at row {i}");
                }
            }
            other => panic!("wrong variant: {other:?}"),
        }
        let (_, took2) = encode_event_catalog(&cat, took, &mut buf).unwrap();
        assert_eq!(took + took2, 11);
        assert_eq!(
            encode_event_catalog(&cat, 11, &mut buf),
            Err(WireError::Range)
        );
    }

    #[test]
    fn weak_mark_round_trips_both_flag_states() {
        let mut buf = [0u8; MAX_EVENT_MSG_BYTES];
        for weak_hit in [false, true] {
            let len = encode_event_weak_mark(0x0141, 0x0087, 0xC3, weak_hit, &mut buf).unwrap();
            assert_eq!(
                decode_event(&buf[..len]).unwrap(),
                EventMsg::WeakMark {
                    cx: 0x0141,
                    cz: 0x0087,
                    mark8: 0xC3,
                    weak_hit,
                }
            );
        }
    }

    #[test]
    fn craft_q_round_trips_and_refuses_bad_jobs() {
        let mut buf = [0u8; MAX_EVENT_MSG_BYTES];
        let jobs = [
            CraftJob {
                recipe: 3,
                remaining: 99,
            },
            CraftJob {
                recipe: 0,
                remaining: 1,
            },
        ];
        let len = encode_event_craft_q(&jobs, 1234, &mut buf).unwrap();
        match decode_event(&buf[..len]).unwrap() {
            EventMsg::CraftQ {
                jobs: got,
                count,
                eta_ticks,
            } => {
                assert_eq!(count, 2);
                assert_eq!(&got[..2], &[(3, 99), (0, 1)]);
                assert_eq!(eta_ticks, 1234);
            }
            other => panic!("wrong variant: {other:?}"),
        }
        // Empty is the "queue cleared" message.
        let len = encode_event_craft_q(&[], 0, &mut buf).unwrap();
        match decode_event(&buf[..len]).unwrap() {
            EventMsg::CraftQ { count, .. } => assert_eq!(count, 0),
            other => panic!("wrong variant: {other:?}"),
        }
        // A dead job, an over-u8 remaining, an out-of-table recipe: Range.
        for bad in [
            CraftJob {
                recipe: 0,
                remaining: 0,
            },
            CraftJob {
                recipe: 0,
                remaining: 256,
            },
            CraftJob {
                recipe: MAX_RECIPES as u16,
                remaining: 1,
            },
        ] {
            assert_eq!(
                encode_event_craft_q(&[bad], 0, &mut buf),
                Err(WireError::Range)
            );
        }
    }

    #[test]
    fn craft_done_and_refused_round_trip() {
        let mut buf = [0u8; MAX_EVENT_MSG_BYTES];
        let len = encode_event_craft_done(9, 3, &mut buf).unwrap();
        assert_eq!(
            decode_event(&buf[..len]).unwrap(),
            EventMsg::CraftDone { item: 9, added: 3 }
        );
        let len = encode_event_craft_refused(4, &mut buf).unwrap();
        assert_eq!(
            decode_event(&buf[..len]).unwrap(),
            EventMsg::CraftRefused { reason: 4 }
        );
    }

    #[test]
    fn recipes_batches_walk_the_table_within_cap() {
        let cc = CraftContent::probe_fixture();
        let mut buf = [0u8; MAX_EVENT_MSG_BYTES];
        let (len, took) = encode_event_recipes(&cc, 0, &mut buf).unwrap();
        assert!(len <= MAX_EVENT_MSG_BYTES);
        assert_eq!(took, 3, "fixture has 3 rows, all fit one batch");
        match decode_event(&buf[..len]).unwrap() {
            EventMsg::Recipes {
                total,
                first,
                count,
                rows,
            } => {
                assert_eq!((total, first, count), (3, 0, 3));
                assert_eq!(rows[0], cc.recipes[0], "decode rebuilds the sim row");
                assert_eq!(rows[1], cc.recipes[1]);
                assert_eq!(rows[2], cc.recipes[2]);
            }
            other => panic!("wrong variant: {other:?}"),
        }
        assert_eq!(
            encode_event_recipes(&cc, 3, &mut buf),
            Err(WireError::Range),
            "cursor past the table refuses"
        );
        // A row the bake would refuse (zero ticks) refuses here.
        let mut bad = cc;
        bad.recipes[1].ticks = 0;
        assert_eq!(
            encode_event_recipes(&bad, 0, &mut buf),
            Err(WireError::Range)
        );
    }

    #[test]
    fn recipes_full_batch_fits_the_cap() {
        // Worst shape: RECIPE_BATCH rows, every input row live at u16 max.
        let mut cc = CraftContent::EMPTY;
        cc.recipe_count = RECIPE_BATCH as u16 + 1;
        for i in 0..cc.recipe_count as usize {
            cc.recipes[i] = RecipeDef {
                output: u16::MAX,
                out_count: 255,
                ticks: 65_535 * sim_core::limits::TICK_HZ,
                station: STATION_MAX,
                // Set, because this is the worst-shape batch test and the
                // bit is one more bit per row: a fixture that left it
                // false would size the batch against a packet the game can
                // actually send one bit wider (research v0).
                blueprint: true,
                n_inputs: MAX_RECIPE_INPUTS as u8,
                inputs: [(u16::MAX, u16::MAX); MAX_RECIPE_INPUTS],
            };
        }
        let mut buf = [0u8; MAX_EVENT_MSG_BYTES];
        let (len, took) = encode_event_recipes(&cc, 0, &mut buf).unwrap();
        assert!(len <= MAX_EVENT_MSG_BYTES);
        assert_eq!(took, RECIPE_BATCH);
        let (_, took2) = encode_event_recipes(&cc, took, &mut buf).unwrap();
        assert_eq!(took + took2, cc.recipe_count as usize);
    }

    #[test]
    fn piece_placed_and_build_refused_round_trip() {
        let mut buf = [0u8; MAX_EVENT_MSG_BYTES];
        let rec = PieceRec {
            cx: 341,
            cz: 682,
            level: 3,
            loc: LOC_EDGE_ZLO,
            row: 17,
            ..PieceRec::default()
        };
        let len = encode_event_piece_placed(&rec, &mut buf).unwrap();
        assert_eq!(
            decode_event(&buf[..len]).unwrap(),
            EventMsg::PiecePlaced { rec }
        );
        // A record the sim could never hold refuses at encode.
        let bad = PieceRec {
            row: MAX_PIECE_DEFS as u8,
            ..rec
        };
        assert_eq!(
            encode_event_piece_placed(&bad, &mut buf),
            Err(WireError::Range)
        );
        let len = encode_event_build_refused(4, &mut buf).unwrap();
        assert_eq!(
            decode_event(&buf[..len]).unwrap(),
            EventMsg::BuildRefused { reason: 4 }
        );
    }

    /// Every plate the sim can produce survives the wire, sign and all
    /// (build plate v1, wire v49).
    ///
    /// **A signed field on a bit writer is where a round trip earns its
    /// keep.** `plate` is written biased and read back by subtracting the
    /// bias, and every way of getting that wrong — the wrong bias, an
    /// unsigned read, a width one bit short — is silent on the value 0, which
    /// is what every fixture in `goldens.rs` carries and what an untouched
    /// column has. A base drawn a storey off the one it is walked on is what
    /// a dropped sign looks like in play.
    #[test]
    fn a_plate_round_trips_through_its_whole_sign() {
        let mut buf = [0u8; MAX_EVENT_MSG_BYTES];
        for plate in -(sim_core::build::PLATE_SINK_MAX_BANDS as i8)
            ..=sim_core::build::PLATE_RISE_MAX_BANDS as i8
        {
            let rec = PieceRec {
                cx: 341,
                cz: 682,
                level: 3,
                loc: LOC_EDGE_ZLO,
                row: 17,
                plate,
                ..PieceRec::default()
            };
            let len = encode_event_piece_placed(&rec, &mut buf).unwrap();
            assert_eq!(
                decode_event(&buf[..len]).unwrap(),
                EventMsg::PiecePlaced { rec },
                "plate {plate} did not survive the wire"
            );
        }
        // And the whole width, past the knobs: the field is deliberately
        // wider than today's limits (`PLATE_BITS` says why), so the values
        // between them and the width must not alias onto legal ones.
        let mut seen = std::collections::BTreeSet::new();
        for plate in -PLATE_BIAS as i8..PLATE_BIAS as i8 {
            let rec = PieceRec {
                cx: 1,
                cz: 1,
                row: 0,
                plate,
                ..PieceRec::default()
            };
            let len = encode_event_piece_placed(&rec, &mut buf).unwrap();
            let EventMsg::PiecePlaced { rec: back } = decode_event(&buf[..len]).unwrap() else {
                panic!("not a placement")
            };
            assert!(
                seen.insert(back.plate),
                "plate {plate} aliased onto another"
            );
        }
        assert_eq!(seen.len(), 1 << PLATE_BITS);
    }

    #[test]
    fn piece_sync_full_batch_fits_the_cap() {
        let mut buf = [0u8; MAX_EVENT_MSG_BYTES];
        let recs: [PieceRec; PIECE_SYNC_BATCH] = core::array::from_fn(|i| PieceRec {
            cx: i as u16 * 31,
            cz: 1023 - i as u16,
            level: (i % MAX_BUILD_LEVELS) as u8,
            loc: (i % 4) as u8,
            row: (i % MAX_PIECE_DEFS) as u8,
            ..PieceRec::default()
        });
        let len = encode_event_piece_sync(true, &recs, &mut buf).unwrap();
        assert!(len <= MAX_EVENT_MSG_BYTES);
        match decode_event(&buf[..len]).unwrap() {
            EventMsg::PieceSync {
                reset,
                recs: got,
                count,
            } => {
                assert!(reset);
                assert_eq!(count as usize, PIECE_SYNC_BATCH);
                assert_eq!(got, recs);
            }
            other => panic!("wrong variant: {other:?}"),
        }
        // Empty is only a message when it resets.
        assert!(encode_event_piece_sync(true, &[], &mut buf).is_ok());
        assert_eq!(
            encode_event_piece_sync(false, &[], &mut buf),
            Err(WireError::Cap)
        );
    }

    #[test]
    fn piece_defs_batches_walk_the_table_within_cap() {
        let bc = BuildContent::probe_fixture();
        let mut buf = [0u8; MAX_EVENT_MSG_BYTES];
        let (len, took) = encode_event_piece_defs(&bc, 0, &mut buf).unwrap();
        assert!(len <= MAX_EVENT_MSG_BYTES);
        // Seven rows since twig v0, against a six-row batch — so the
        // fixture no longer fits one message and this test walks the
        // cursor, which it never did before and which is the thing the
        // batching exists for.
        assert_eq!(took, PIECE_DEFS_BATCH, "the first batch is full");
        match decode_event(&buf[..len]).unwrap() {
            EventMsg::PieceDefs {
                total,
                first,
                count,
                rows,
            } => {
                assert_eq!((total, first, count), (7, 0, PIECE_DEFS_BATCH as u8));
                assert_eq!(rows[0], bc.pieces[0], "decode rebuilds the sim row");
                assert_eq!(rows[PIECE_DEFS_BATCH - 1], bc.pieces[PIECE_DEFS_BATCH - 1]);
            }
            other => panic!("wrong variant: {other:?}"),
        }
        // The remainder rides the second batch, and the cursor lands it at
        // the row the first one stopped on.
        let (len, took) = encode_event_piece_defs(&bc, PIECE_DEFS_BATCH, &mut buf).unwrap();
        assert_eq!(took, 1, "one row left over");
        match decode_event(&buf[..len]).unwrap() {
            EventMsg::PieceDefs {
                total,
                first,
                count,
                rows,
            } => {
                assert_eq!((total, first, count), (7, PIECE_DEFS_BATCH as u8, 1));
                assert_eq!(rows[0], bc.pieces[PIECE_DEFS_BATCH]);
            }
            other => panic!("wrong variant: {other:?}"),
        }
        assert_eq!(
            encode_event_piece_defs(&bc, 7, &mut buf),
            Err(WireError::Range),
            "cursor past the table refuses"
        );
        // A row the bake would refuse (hp 0) refuses here.
        let mut bad = bc;
        bad.pieces[1].hp = 0;
        assert_eq!(
            encode_event_piece_defs(&bad, 0, &mut buf),
            Err(WireError::Range)
        );
        // The full 18-row alpha shape drips in three batches.
        let mut full = BuildContent::EMPTY;
        full.piece_count = 18;
        for i in 0..18 {
            full.pieces[i] = PieceDef {
                shape: (i % 6) as u8,
                material: (i % 4) as u8,
                hp: u16::MAX,
                n_costs: MAX_PIECE_COSTS as u8,
                costs: [(u16::MAX, u16::MAX); MAX_PIECE_COSTS],
            };
        }
        let (len, took) = encode_event_piece_defs(&full, 0, &mut buf).unwrap();
        assert!(len <= MAX_EVENT_MSG_BYTES);
        assert_eq!(took, PIECE_DEFS_BATCH);
        let (_, took2) = encode_event_piece_defs(&full, took, &mut buf).unwrap();
        let (_, took3) = encode_event_piece_defs(&full, took + took2, &mut buf).unwrap();
        assert_eq!(took + took2 + took3, 18);
    }

    #[test]
    fn deploy_placed_sync_and_refused_round_trip() {
        let mut buf = [0u8; MAX_EVENT_MSG_BYTES];
        let rec = DeployRec {
            cx: 341,
            cz: 682,
            level: 1,
            loc: sim_core::build::LOC_PLANE,
            row: 7,
            ..DeployRec::default()
        };
        let len = encode_event_deploy_placed(&rec, &mut buf).unwrap();
        assert_eq!(
            decode_event(&buf[..len]).unwrap(),
            EventMsg::DeployPlaced { rec },
            "owner/hp/uh stay sim-side: decode carries defaults"
        );
        // A record the sim could never hold refuses at encode.
        let bad = DeployRec {
            row: MAX_DEPLOY_DEFS as u8,
            ..rec
        };
        assert_eq!(
            encode_event_deploy_placed(&bad, &mut buf),
            Err(WireError::Range)
        );

        // A full sync batch fits the cap; empty only resets.
        let recs: [DeployRec; DEPLOY_SYNC_BATCH] = core::array::from_fn(|i| DeployRec {
            cx: i as u16 * 41,
            cz: 1023 - i as u16,
            level: (i % MAX_BUILD_LEVELS) as u8,
            loc: (i % 4) as u8,
            row: (i % MAX_DEPLOY_DEFS) as u8,
            ..DeployRec::default()
        });
        let len = encode_event_deploy_sync(true, &recs, &mut buf).unwrap();
        assert!(len <= MAX_EVENT_MSG_BYTES);
        match decode_event(&buf[..len]).unwrap() {
            EventMsg::DeploySync {
                reset,
                recs: got,
                count,
            } => {
                assert!(reset);
                assert_eq!(count as usize, DEPLOY_SYNC_BATCH);
                assert_eq!(got, recs);
            }
            other => panic!("wrong variant: {other:?}"),
        }
        assert!(encode_event_deploy_sync(true, &[], &mut buf).is_ok());
        assert_eq!(
            encode_event_deploy_sync(false, &[], &mut buf),
            Err(WireError::Cap)
        );

        let len = encode_event_deploy_refused(7, &mut buf).unwrap();
        assert_eq!(
            decode_event(&buf[..len]).unwrap(),
            EventMsg::DeployRefused { reason: 7 }
        );
    }

    #[test]
    fn deploy_defs_batches_walk_the_table_within_cap() {
        let dc = DeployContent::probe_fixture();
        let mut buf = [0u8; MAX_EVENT_MSG_BYTES];
        let (len, took) = encode_event_deploy_defs(&dc, 0, &mut buf).unwrap();
        assert!(len <= MAX_EVENT_MSG_BYTES);
        assert_eq!(took, 8, "fixture has 8 rows, all fit one batch");
        match decode_event(&buf[..len]).unwrap() {
            EventMsg::DeployDefs {
                total,
                first,
                count,
                rows,
            } => {
                assert_eq!((total, first, count), (8, 0, 8));
                assert_eq!(rows[0], dc.defs[0], "decode rebuilds the sim row");
                assert_eq!(rows[3], dc.defs[3]);
                // The oven row (oven v0), the code lock (lock v1), the
                // recycler (recycler v0) and the research table (research
                // v0) — the fixture's newest, and therefore the ones a
                // batch walk would drop off the end. The last two carry
                // archetypes that did not fit the old three-bit field, so
                // this is where an `ARCH_BITS` left at 3 would surface.
                assert_eq!(rows[4], dc.defs[4]);
                assert_eq!(rows[5], dc.defs[5]);
                assert_eq!(rows[6], dc.defs[6]);
                assert_eq!(rows[7], dc.defs[7]);
            }
            other => panic!("wrong variant: {other:?}"),
        }
        assert_eq!(
            encode_event_deploy_defs(&dc, 8, &mut buf),
            Err(WireError::Range),
            "cursor past the table refuses"
        );
        // A row the bake would refuse (hp 0) refuses here.
        let mut bad = dc;
        bad.defs[1].hp = 0;
        assert_eq!(
            encode_event_deploy_defs(&bad, 0, &mut buf),
            Err(WireError::Range)
        );
        // A cost count past the table refuses too — the decoder bounds it
        // at `MAX_DEPLOY_COSTS`, so an encoder that wrote more would emit
        // a message its own reader calls malformed.
        let mut wide = dc;
        wide.defs[0].n_costs = MAX_DEPLOY_COSTS as u8 + 1;
        assert_eq!(
            encode_event_deploy_defs(&wide, 0, &mut buf),
            Err(WireError::Range)
        );

        // The worst batch this subtype can emit still fits the cap: a full
        // `DEPLOY_DEFS_BATCH` of rows each carrying the maximum cost rows.
        // The price rows are new in v21 and they are the widest part of the
        // row, so the cap needs asserting rather than assuming.
        let mut full = DeployContent::EMPTY;
        full.def_count = DEPLOY_DEFS_BATCH as u16;
        for (n, row) in full.defs.iter_mut().take(DEPLOY_DEFS_BATCH).enumerate() {
            *row = DeployDef {
                arch: sim_core::deploy::ARCH_BOX,
                placement: sim_core::deploy::PLACE_ANY,
                hp: u16::MAX,
                item: u16::MAX,
                n_costs: MAX_DEPLOY_COSTS as u8,
                costs: [(u16::MAX, u16::MAX); MAX_DEPLOY_COSTS],
            };
            let _ = n;
        }
        let (len, took) = encode_event_deploy_defs(&full, 0, &mut buf).unwrap();
        assert_eq!(took, DEPLOY_DEFS_BATCH, "a full batch rides");
        assert!(
            len <= MAX_EVENT_MSG_BYTES,
            "a full priced deploy-defs batch is {len} B against a \
             {MAX_EVENT_MSG_BYTES} B cap"
        );
        match decode_event(&buf[..len]).unwrap() {
            EventMsg::DeployDefs { rows, .. } => {
                assert_eq!(
                    rows[DEPLOY_DEFS_BATCH - 1],
                    full.defs[DEPLOY_DEFS_BATCH - 1]
                )
            }
            other => panic!("wrong variant: {other:?}"),
        }
        // The 16-row cap shape drips in two batches.
        let mut full = DeployContent::EMPTY;
        full.def_count = MAX_DEPLOY_DEFS as u16;
        for i in 0..MAX_DEPLOY_DEFS {
            full.defs[i] = DeployDef {
                arch: (i % 10) as u8,
                placement: (i % 4) as u8,
                hp: u16::MAX,
                item: u16::MAX,
                ..DeployDef::INERT
            };
        }
        let (len, took) = encode_event_deploy_defs(&full, 0, &mut buf).unwrap();
        assert!(len <= MAX_EVENT_MSG_BYTES);
        assert_eq!(took, DEPLOY_DEFS_BATCH);
        let (_, took2) = encode_event_deploy_defs(&full, took, &mut buf).unwrap();
        assert_eq!(took + took2, MAX_DEPLOY_DEFS);
    }

    #[test]
    fn removals_and_stock_round_trip() {
        let mut buf = [0u8; MAX_EVENT_MSG_BYTES];
        for piece in [true, false] {
            let len = encode_event_removed(piece, 341, 682, 2, LOC_EDGE_ZLO, &mut buf).unwrap();
            let want = if piece {
                EventMsg::PieceRemoved {
                    cx: 341,
                    cz: 682,
                    level: 2,
                    loc: LOC_EDGE_ZLO,
                }
            } else {
                EventMsg::DeployRemoved {
                    cx: 341,
                    cz: 682,
                    level: 2,
                    loc: LOC_EDGE_ZLO,
                }
            };
            assert_eq!(decode_event(&buf[..len]).unwrap(), want);
        }
        assert_eq!(
            encode_event_removed(true, 1024, 0, 0, 0, &mut buf),
            Err(WireError::Range)
        );

        // The raid's progress message, both stores (the row field is a
        // different width in each), and every out-of-range face of it.
        for deploy in [false, true] {
            let row = if deploy { 15 } else { 31 };
            let len =
                encode_event_struct_hit(deploy, 341, 682, 2, LOC_EDGE_ZLO, row, 40, 710, &mut buf)
                    .unwrap();
            assert_eq!(
                decode_event(&buf[..len]).unwrap(),
                EventMsg::StructHit {
                    deploy,
                    cx: 341,
                    cz: 682,
                    level: 2,
                    loc: LOC_EDGE_ZLO,
                    damage: 40,
                    left: 710,
                }
            );
        }
        assert_eq!(
            encode_event_struct_hit(false, 1024, 0, 0, 0, 0, 1, 1, &mut buf),
            Err(WireError::Range),
            "cell past the grid"
        );
        assert_eq!(
            encode_event_struct_hit(false, 0, 0, MAX_BUILD_LEVELS as u8, 0, 0, 1, 1, &mut buf),
            Err(WireError::Range),
            "level past the grid"
        );
        // The loc bound is the STORE's since v40: the piece side reaches
        // the diagonals, the deploy side still ends at the straight edges
        // — a deploy hit on a triangle address is a forged address.
        assert_eq!(
            encode_event_struct_hit(
                false,
                0,
                0,
                0,
                sim_core::build::LOC_DIAG_B + 1,
                0,
                1,
                1,
                &mut buf
            ),
            Err(WireError::Range),
            "loc past the piece store's ten"
        );
        assert_eq!(
            encode_event_struct_hit(
                true,
                0,
                0,
                0,
                sim_core::build::LOC_TRI_XLO_ZLO,
                0,
                1,
                1,
                &mut buf
            ),
            Err(WireError::Range),
            "a deploy hit never lands on a triangle"
        );
        assert_eq!(
            encode_event_struct_hit(true, 0, 0, 0, 0, 16, 1, 1, &mut buf),
            Err(WireError::Range),
            "a deploy row past the 4-bit field"
        );
        assert_eq!(
            encode_event_struct_hit(false, 0, 0, 0, 0, 0, 40, 0, &mut buf),
            Err(WireError::Range),
            "zero hp left is a removal, and removals have their own subtype"
        );

        // Stock at the row cap round-trips; past it refuses.
        let rows = [(0u16, 2_000u32), (5, 0), (9, 123_456), (63, u32::MAX)];
        let len = encode_event_stock(341, 682, 0, &rows, &mut buf).unwrap();
        match decode_event(&buf[..len]).unwrap() {
            EventMsg::Stock {
                cx,
                cz,
                level,
                rows: got,
                count,
            } => {
                assert_eq!((cx, cz, level), (341, 682, 0));
                assert_eq!(count as usize, HEARTH_STOCK_ROWS);
                assert_eq!(got, rows);
            }
            other => panic!("wrong variant: {other:?}"),
        }
        let too_many = [(0u16, 0u32); HEARTH_STOCK_ROWS + 1];
        assert_eq!(
            encode_event_stock(0, 0, 0, &too_many, &mut buf),
            Err(WireError::Cap)
        );
    }

    /// The container sync's shape, both ways, plus every refusal it owes.
    ///
    /// The refusals are the point rather than the round trip. This message
    /// has a field whose value changes what the *other* fields are allowed
    /// to be — `kind == CONT_SELF` is a close, and a close carries no
    /// handle and no slots — and that is exactly the shape that decays
    /// into "well, the client ignores those anyway". So each illegal
    /// combination is asserted here on both the encoder and the decoder;
    /// a future edit that relaxes one end alone fails on the other.
    #[test]
    fn cont_sync_round_trips_and_refuses_every_illegal_shape() {
        let mut buf = [0u8; MAX_EVENT_MSG_BYTES];
        let rows = [
            InvSlot {
                slot: 0,
                stack: ItemStack {
                    item: 9,
                    count: 4,
                    cond: 0,
                },
            },
            InvSlot {
                slot: 11,
                stack: ItemStack {
                    item: 21,
                    count: 60,
                    cond: 7_500,
                },
            },
        ];

        // A box diff. Slot 11 is the last a `BOX_SLOTS` container has, so
        // the tight per-kind bound is exercised at its edge rather than in
        // its middle.
        let len = encode_event_cont_sync(CONT_BOX, 0x0011_2233, false, &rows, &mut buf).unwrap();
        match decode_event(&buf[..len]).unwrap() {
            EventMsg::ContSync {
                kind,
                cont,
                reset,
                slots,
                count,
            } => {
                assert_eq!(
                    (kind, cont, reset, count),
                    (CONT_BOX, 0x0011_2233, false, 2)
                );
                assert_eq!(&slots[..2], &rows);
            }
            other => panic!("wrong variant: {other:?}"),
        }

        // A bag reset carrying the widest slot index there is.
        let wide = [InvSlot {
            slot: (INV_SLOTS - 1) as u8,
            stack: ItemStack {
                item: 3,
                count: 1,
                cond: 42,
            },
        }];
        let len = encode_event_cont_sync(CONT_BAG, 7, true, &wide, &mut buf).unwrap();
        match decode_event(&buf[..len]).unwrap() {
            EventMsg::ContSync {
                kind, reset, slots, ..
            } => {
                assert_eq!((kind, reset), (CONT_BAG, true));
                assert_eq!(slots[0], wide[0]);
            }
            other => panic!("wrong variant: {other:?}"),
        }

        // The close, and the empty-batch contract it rests on.
        let len = encode_event_cont_sync(CONT_SELF, 0, true, &[], &mut buf).unwrap();
        match decode_event(&buf[..len]).unwrap() {
            EventMsg::ContSync {
                kind, cont, count, ..
            } => assert_eq!((kind, cont, count), (CONT_SELF, 0, 0)),
            other => panic!("wrong variant: {other:?}"),
        }

        // A kind past the live set.
        assert_eq!(
            encode_event_cont_sync(CONT_MAX + 1, 1, true, &[], &mut buf),
            Err(WireError::Range)
        );
        // A close that names a container, holds slots, or is not a reset —
        // the three ways the one cross-field rule can be broken.
        assert_eq!(
            encode_event_cont_sync(CONT_SELF, 5, true, &[], &mut buf),
            Err(WireError::Range)
        );
        assert_eq!(
            encode_event_cont_sync(CONT_SELF, 0, true, &rows, &mut buf),
            Err(WireError::Range)
        );
        assert_eq!(
            encode_event_cont_sync(CONT_SELF, 0, false, &[], &mut buf),
            Err(WireError::Range)
        );
        // An empty batch that is not a reset says nothing.
        assert_eq!(
            encode_event_cont_sync(CONT_BAG, 1, false, &[], &mut buf),
            Err(WireError::Cap)
        );
        // Past the batch.
        let too_many = [InvSlot::default(); CONT_SYNC_BATCH + 1];
        assert_eq!(
            encode_event_cont_sync(CONT_BAG, 1, true, &too_many, &mut buf),
            Err(WireError::Cap)
        );
        // A slot inside `INV_SLOTS` but past **this kind's** container.
        // This is the tightness the action lane deliberately does not have
        // (see `encode_event_cont_sync`), so it needs its own assertion —
        // the same slot is legal one line down under a bag.
        let past_box = [InvSlot {
            slot: BOX_SLOTS as u8,
            stack: ItemStack {
                item: 1,
                count: 1,
                cond: 0,
            },
        }];
        assert_eq!(
            encode_event_cont_sync(CONT_BOX, 1, true, &past_box, &mut buf),
            Err(WireError::Range)
        );
        assert!(encode_event_cont_sync(CONT_BAG, 1, true, &past_box, &mut buf).is_ok());

        // And the decoder refuses the same shapes off the wire, because a
        // client that trusted the encoder's checks would be trusting a
        // server it has no reason to. Forged by hand: a close whose handle
        // is not zero.
        let mut w = BitWriter::new(&mut buf);
        w.write(KIND_EVENT, KIND_BITS).unwrap();
        w.write(SUB_CONT_SYNC, SUB_BITS).unwrap();
        w.write(CONT_SELF as u32, CONT_KIND_BITS).unwrap();
        w.write(0xDEAD_BEEF, 32).unwrap();
        w.write_bit(true).unwrap();
        w.write(0, CONT_COUNT_BITS).unwrap();
        let len = w.finish();
        assert_eq!(decode_event(&buf[..len]), Err(WireError::Malformed));

        // A box batch whose slot is past `BOX_SLOTS`.
        let mut w = BitWriter::new(&mut buf);
        w.write(KIND_EVENT, KIND_BITS).unwrap();
        w.write(SUB_CONT_SYNC, SUB_BITS).unwrap();
        w.write(CONT_BOX as u32, CONT_KIND_BITS).unwrap();
        w.write(1, 32).unwrap();
        w.write_bit(true).unwrap();
        w.write(1, CONT_COUNT_BITS).unwrap();
        w.write(BOX_SLOTS as u32, INV_SLOT_BITS).unwrap();
        w.write(1, 16).unwrap();
        w.write(1, 16).unwrap();
        let len = w.finish();
        assert_eq!(decode_event(&buf[..len]), Err(WireError::Malformed));

        // A count past the batch — forgeable because the width holds 63
        // and the batch is 30.
        let mut w = BitWriter::new(&mut buf);
        w.write(KIND_EVENT, KIND_BITS).unwrap();
        w.write(SUB_CONT_SYNC, SUB_BITS).unwrap();
        w.write(CONT_BAG as u32, CONT_KIND_BITS).unwrap();
        w.write(1, 32).unwrap();
        w.write_bit(true).unwrap();
        w.write(CONT_SYNC_BATCH as u32 + 1, CONT_COUNT_BITS)
            .unwrap();
        let len = w.finish();
        assert_eq!(decode_event(&buf[..len]), Err(WireError::Malformed));
    }

    /// A whole container fits one message — the claim `CONT_SYNC_BATCH`
    /// makes when it refuses to be a drip constant. If this ever fails,
    /// the answer is a cursor, not a wider cap.
    #[test]
    fn a_full_container_fits_one_message() {
        let mut buf = [0u8; MAX_EVENT_MSG_BYTES];
        let mut rows = [InvSlot::default(); CONT_SYNC_BATCH];
        for (i, r) in rows.iter_mut().enumerate() {
            *r = InvSlot {
                slot: i as u8,
                stack: ItemStack {
                    item: (i as u16) + 1,
                    count: u16::MAX,
                    cond: u16::MAX,
                },
            };
        }
        let len = encode_event_cont_sync(CONT_BAG, u32::MAX, true, &rows, &mut buf).unwrap();
        assert!(
            len <= MAX_EVENT_MSG_BYTES,
            "a full container is {len} B against a {MAX_EVENT_MSG_BYTES} B cap"
        );
        assert_eq!(CONT_SYNC_BATCH, INV_SLOTS, "the widest container is a bag");
        match decode_event(&buf[..len]).unwrap() {
            EventMsg::ContSync { slots, count, .. } => {
                assert_eq!(count as usize, CONT_SYNC_BATCH);
                assert_eq!(slots, rows);
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn trailing_garbage_and_unknown_subtype_are_malformed() {
        let mut buf = [0u8; MAX_EVENT_MSG_BYTES];
        let len = encode_event_gather(1, 2, &mut buf).unwrap();
        assert_eq!(
            decode_event(&buf[..len + 1]),
            Err(WireError::Malformed),
            "spare byte after a valid message must fail the strict tail"
        );
        // kind EVENT + the first unused subtype — 30 became struct-hit,
        // 31–33 the survival clock's three, 34 the drink, 35 the respawn
        // and 36–37 the move pair, so this moves up with every new
        // subtype, exactly as intended. The 5 → 6 widening at v13 leaves
        // 26 codes free after it, so the probe keeps one of its own for a
        // long time.
        //
        // Off `SUB_MAX` and not off whichever subtype was last when this
        // was written: as `SUB_RESPAWN + 1` it had already drifted onto a
        // live code, which turns "the decoder refuses unknown subtypes"
        // into "the decoder refuses this one message with a short payload".
        const UNUSED_SUB: u32 = SUB_MAX + 1;
        const { assert!(UNUSED_SUB < 1 << SUB_BITS, "the probe must fit the field") };
        let raw = [
            (KIND_EVENT | (UNUSED_SUB << KIND_BITS)) as u8,
            (UNUSED_SUB >> (8 - KIND_BITS)) as u8,
        ];
        assert_eq!(decode_event(&raw[..2]), Err(WireError::Malformed));
        // And the top of the widened field is unknown too — a decoder that
        // masked the new bit off would fall into a live subtype here.
        let top = (1u32 << SUB_BITS) - 1;
        let raw = [
            (KIND_EVENT | (top << KIND_BITS)) as u8,
            (top >> (8 - KIND_BITS)) as u8,
        ];
        assert_eq!(decode_event(&raw[..2]), Err(WireError::Malformed));
    }
}

/// The wire's value domains, gated against the sim's — the half wall 6
/// does not reach.
///
/// `test_protocol_golden` pins **layout**: which field, how wide, in what
/// order. Every constant below is a *domain* — which values a field of
/// already-fixed width is allowed to carry — and a domain can drift
/// without moving one byte of layout. `sim-core` grows a ninth archetype
/// or a fourth death cause, the encoder's range check still reads the old
/// bound, and the golden is green because the packet it pins never
/// contained the new value. The only witness is `Err(Range)` at runtime,
/// on the exact path a player notices and a test suite does not.
///
/// That is not a hypothetical; it is a judged **FAIL** on 2026-08-05.
/// `DEATH_BY_ARROW = 3` landed against `DEATH_CAUSE_MAX = 2`, every arrow
/// kill failed to encode, the server counted the range error and sent
/// nothing, and the victim's client never learned it had died. Golden
/// green, replay green, clippy green. `reference/FINDINGS.md` §1 measured
/// the same class in the reference ecosystem — ~27 Oxide commits
/// correcting a payload that had already shipped wrong, against an
/// `MSILHash` that is the exact analogue of our golden and caught none of
/// them.
///
/// So this reads the sim's constant blocks as text and checks each domain
/// still fits the field that carries it. It reads **every module of
/// `sim-core`**, not the one file a domain is declared in: the first cut
/// of this gate read one file per domain, and a
/// `pub const DEATH_BY_ARROW: u8 = 3;` appended to `combat.rs` left all
/// of it green while `encode_event_death` still returned `Err(Range)` —
/// the identical failure, one module over, with the gate written to catch
/// it watching the wrong file. A domain is the set of values the sim can
/// emit, and the sim is the crate.
///
/// Parsing source is the deliberate
/// choice `event_roles.rs` documents and it is the same reason here: the
/// fact under assertion is *what the constant block contains*, and no
/// amount of importing can see a constant the importer was never told
/// about. `use sim_core::deploy::*` would not notice `ARCH_TURRET`;
/// reading the block does.
#[cfg(test)]
mod wire_domains {
    use super::*;

    /// One coupled pair: a `sim-core` value domain, and the wire field
    /// this module spends on it.
    struct Domain {
        /// Named in the failure, so a red gate says which pair drifted.
        what: &'static str,
        /// Where the domain is declared, and where the width is.
        sim_site: &'static str,
        wire_site: &'static str,
        /// The module this domain is declared in, e.g. `"world.rs"`.
        ///
        /// Not where the gate *looks* — it reads every module in
        /// `SOURCES` — but where the domain is allowed to live. Scraping
        /// one file was the hole: on 2026-08-05 a stray
        /// `pub const DEATH_BY_ARROW: u8 = 3;` in `combat.rs` left all
        /// three checks below green while `encode_event_death` still
        /// refused cause 3, which is the original failure one module
        /// over. The crate-wide read catches the value; this field is
        /// what makes the red name the file.
        home: &'static str,
        /// `pub const <prefix>NAME<ty> = <literal>;` — the shape scraped.
        /// The type is part of the match on purpose: it is what keeps
        /// `craft.rs`'s `STATION_RADIUS_M: f32 = 5.0` out of `STATION_*`,
        /// a real collision and the only cross-type one in the ten.
        prefix: &'static str,
        ty: &'static str,
        /// Members that name the ledger rather than sit in it. These are
        /// declared as an alias (`CONT_MAX = CONT_WORLD` today), so they
        /// would fail the literal parse below — skipping them is not a
        /// convenience, it is the difference between a gate and a panic.
        exempt: &'static [&'static str],
        /// Below this the parser has stopped seeing the block and the
        /// gate is reading nothing. A gate that silently stops looking is
        /// worse than one that fails.
        min_members: u32,
        /// The width the wire spends, in bits.
        bits: u32,
        /// The largest value the sim can emit **today**, pinned.
        ///
        /// The fit check below is necessary and not sufficient. A fourth
        /// death cause is `3`, and `3` fits `DEATH_CAUSE_BITS` — so
        /// widening the domain into spare headroom passes every numeric
        /// check while changing what a bit pattern *means*: a value both
        /// ends previously refused as forged is now a live fact. `lib.rs`
        /// is explicit that this is a wire change — "a widened meaning is
        /// a wire change even when the layout is byte-identical, and
        /// PROTO_VER is the only thing that catches a mismatched build" —
        /// and no golden can see it, because the golden pins packets that
        /// never carried the new value. Pinning the maximum is what makes
        /// the widening require a sentence from whoever does it.
        live_max: u32,
    }

    /// One `sim-core` module, read at compile time.
    struct Module {
        file: &'static str,
        src: &'static str,
    }

    /// Every module of `sim-core`, so a domain member cannot hide in one
    /// the table forgot.
    ///
    /// `include_str!` needs a literal path, so this list cannot be a
    /// glob — which makes a stale list the obvious next hole, and
    /// `the_source_table_covers_the_whole_crate` below is what closes it:
    /// it reads `lib.rs`'s own `mod` declarations and fails if this table
    /// and that list disagree in either direction.
    const SOURCES: &[Module] = &[
        Module {
            file: "lib.rs",
            src: include_str!("../../sim-core/src/lib.rs"),
        },
        Module {
            file: "backpack.rs",
            src: include_str!("../../sim-core/src/backpack.rs"),
        },
        Module {
            file: "bots.rs",
            src: include_str!("../../sim-core/src/bots.rs"),
        },
        Module {
            file: "build.rs",
            src: include_str!("../../sim-core/src/build.rs"),
        },
        Module {
            file: "charge.rs",
            src: include_str!("../../sim-core/src/charge.rs"),
        },
        Module {
            file: "persist.rs",
            src: include_str!("../../sim-core/src/persist.rs"),
        },
        Module {
            file: "research.rs",
            src: include_str!("../../sim-core/src/research.rs"),
        },
        Module {
            file: "pitch_lut.rs",
            src: include_str!("../../sim-core/src/pitch_lut.rs"),
        },
        Module {
            file: "ranged.rs",
            src: include_str!("../../sim-core/src/ranged.rs"),
        },
        Module {
            file: "claim.rs",
            src: include_str!("../../sim-core/src/claim.rs"),
        },
        Module {
            file: "collide.rs",
            src: include_str!("../../sim-core/src/collide.rs"),
        },
        Module {
            file: "combat.rs",
            src: include_str!("../../sim-core/src/combat.rs"),
        },
        Module {
            file: "craft.rs",
            src: include_str!("../../sim-core/src/craft.rs"),
        },
        Module {
            file: "deploy.rs",
            src: include_str!("../../sim-core/src/deploy.rs"),
        },
        Module {
            file: "fmath.rs",
            src: include_str!("../../sim-core/src/fmath.rs"),
        },
        Module {
            file: "gather.rs",
            src: include_str!("../../sim-core/src/gather.rs"),
        },
        Module {
            file: "input.rs",
            src: include_str!("../../sim-core/src/input.rs"),
        },
        Module {
            file: "inventory.rs",
            src: include_str!("../../sim-core/src/inventory.rs"),
        },
        Module {
            file: "limits.rs",
            src: include_str!("../../sim-core/src/limits.rs"),
        },
        Module {
            file: "lock.rs",
            src: include_str!("../../sim-core/src/lock.rs"),
        },
        Module {
            file: "roster.rs",
            src: include_str!("../../sim-core/src/roster.rs"),
        },
        Module {
            file: "loot.rs",
            src: include_str!("../../sim-core/src/loot.rs"),
        },
        Module {
            file: "mob.rs",
            src: include_str!("../../sim-core/src/mob.rs"),
        },
        Module {
            file: "movement.rs",
            src: include_str!("../../sim-core/src/movement.rs"),
        },
        Module {
            file: "occupy.rs",
            src: include_str!("../../sim-core/src/occupy.rs"),
        },
        Module {
            file: "oven.rs",
            src: include_str!("../../sim-core/src/oven.rs"),
        },
        Module {
            file: "probe.rs",
            src: include_str!("../../sim-core/src/probe.rs"),
        },
        Module {
            file: "rng.rs",
            src: include_str!("../../sim-core/src/rng.rs"),
        },
        Module {
            file: "spent.rs",
            src: include_str!("../../sim-core/src/spent.rs"),
        },
        Module {
            file: "survival.rs",
            src: include_str!("../../sim-core/src/survival.rs"),
        },
        Module {
            file: "terrain.rs",
            src: include_str!("../../sim-core/src/terrain.rs"),
        },
        Module {
            file: "world.rs",
            src: include_str!("../../sim-core/src/world.rs"),
        },
        Module {
            file: "worldcont.rs",
            src: include_str!("../../sim-core/src/worldcont.rs"),
        },
        Module {
            file: "worldsave.rs",
            src: include_str!("../../sim-core/src/worldsave.rs"),
        },
        Module {
            file: "yaw_lut.rs",
            src: include_str!("../../sim-core/src/yaw_lut.rs"),
        },
    ];

    /// All twelve domains this module bounds. Widths are the private consts
    /// above, so this table cannot drift from the encoder — it *is* the
    /// encoder's constants.
    ///
    /// Note the two shapes of headroom, both deliberate and both stated
    /// so a later reader does not "tidy" one into the other. `PLACE_*`
    /// saturates its two bits exactly (0..=3, all live), so no value is
    /// forgeable and the decoder needs no domain check. `REFUSE_C_*` and
    /// `BAG_GONE_*` do **not** saturate — and since 2026-08-17 both ends
    /// refuse the slack against the sim's own `*_MAX` (the wire act
    /// NOW.md §5b owed; `the_refusal_slack_refuses_at_both_ends` is the
    /// gate, and the narrowing rule at `PROTO_VER` is why no version
    /// turned for it).
    const DOMAINS: &[Domain] = &[
        Domain {
            what: "death cause",
            sim_site: "world.rs DEATH_BY_*",
            wire_site: "DEATH_CAUSE_BITS",
            home: "world.rs",
            prefix: "pub const DEATH_BY_",
            ty: ": u8 = ",
            exempt: &["MAX"],
            min_members: 6,
            bits: DEATH_CAUSE_BITS,
            // 4 = DEATH_BY_MOB and 5 = DEATH_BY_CHARGE, the meanings the
            // v36 field widening was minted for — pin moved in the same
            // merge window as the bump, once per cause.
            live_max: 5,
        },
        Domain {
            what: "move refusal",
            sim_site: "inventory.rs REFUSE_M_*",
            wire_site: "REFUSE_M_BITS",
            home: "inventory.rs",
            prefix: "pub const REFUSE_M_",
            ty: ": u32 = ",
            exempt: &["MAX"],
            min_members: 8,
            bits: REFUSE_M_BITS,
            // 8 -> 9 at wire v51 (armor v1): `REFUSE_M_WEAR` is the
            // reason a wear slot gives for "that is not what goes here",
            // and the pin fired on it exactly as its message promised.
            live_max: 9,
        },
        Domain {
            what: "consume refusal",
            sim_site: "survival.rs REFUSE_C_*",
            wire_site: "REFUSE_C_BITS",
            home: "survival.rs",
            prefix: "pub const REFUSE_C_",
            ty: ": u32 = ",
            // `REFUSE_C_MAX` names the ledger rather than sitting in it.
            // It is a literal upstream only because this exempt row did
            // not exist when it landed (`survival.rs` says so); with the
            // row here, `sim-core` is free to alias it like `DEATH_BY_MAX`,
            // and this module reads the constant itself — not the scrape —
            // wherever it bounds the field.
            exempt: &["MAX"],
            min_members: 3,
            bits: REFUSE_C_BITS,
            live_max: 3,
        },
        Domain {
            what: "gather refusal",
            sim_site: "gather.rs REFUSE_G_*",
            wire_site: "REFUSE_G_BITS",
            home: "gather.rs",
            prefix: "pub const REFUSE_G_",
            ty: ": u32 = ",
            exempt: &["MAX"],
            min_members: 2,
            bits: REFUSE_G_BITS,
            live_max: 2,
        },
        Domain {
            what: "build refusal",
            sim_site: "build.rs REFUSE_B_*",
            wire_site: "REFUSE_B_BITS",
            home: "build.rs",
            prefix: "pub const REFUSE_B_",
            ty: ": u32 = ",
            exempt: &[],
            min_members: 15,
            bits: REFUSE_B_BITS,
            live_max: 14,
        },
        Domain {
            what: "container kind",
            sim_site: "inventory.rs CONT_*",
            wire_site: "CONT_KIND_BITS",
            home: "inventory.rs",
            prefix: "pub const CONT_",
            ty: ": u8 = ",
            exempt: &["MAX"],
            min_members: 5,
            bits: CONT_KIND_BITS,
            // Moved 2 -> 3 at wire v37 (world containers v0), which is
            // the case this pin was written for: no field widened and no
            // fixture's bytes moved, but value 3 stopped being forged and
            // started being the crate on the haven pad. That saturated
            // the domain, and the note here said the next kind could not
            // land without a widening.
            //
            // It did, at v51 (armor v1): `CONT_WEAR` is 4, the field went
            // 2 -> 3 bits, and every fixture carrying a container address
            // re-keyed. So this pin has now fired for real, in both of
            // its jobs — the fit assert refused the fifth kind under two
            // bits, and this `live_max` refused it under three until the
            // widening was deliberate. Values 5..7 are forgeable and
            // refuse at both ends, which is the posture the field lost at
            // v37 and has back.
            live_max: 4,
        },
        Domain {
            what: "piece shape",
            sim_site: "build.rs SHAPE_*",
            wire_site: "SHAPE_BITS",
            home: "build.rs",
            prefix: "pub const SHAPE_",
            ty: ": u8 = ",
            exempt: &[],
            min_members: 11,
            bits: SHAPE_BITS,
            // Moved 5 -> 7 at wire v38 (catalogue v1), saturating the
            // 3-bit field exactly as this pin then warned; v40 is the
            // widening it priced — `SHAPE_BITS` 3 -> 4 for the three
            // triangle shapes (`reference/BUILDING.md` §9.14). 11 of 16
            // values live; the decoder range-checks the tail.
            live_max: 10,
        },
        Domain {
            what: "piece material",
            sim_site: "build.rs MAT_*",
            wire_site: "MATERIAL_BITS",
            home: "build.rs",
            prefix: "pub const MAT_",
            ty: ": u8 = ",
            exempt: &[],
            min_members: 4,
            bits: MATERIAL_BITS,
            live_max: 3,
        },
        Domain {
            what: "research refusal",
            sim_site: "research.rs REFUSE_R_*",
            wire_site: "RESEARCH_REFUSE_BITS",
            home: "research.rs",
            prefix: "pub const REFUSE_R_",
            ty: ": u32 = ",
            exempt: &["MAX"],
            min_members: 7,
            bits: RESEARCH_REFUSE_BITS,
            live_max: 6,
        },
        Domain {
            what: "deploy archetype",
            sim_site: "deploy.rs ARCH_*",
            wire_site: "ARCH_BITS",
            home: "deploy.rs",
            prefix: "pub const ARCH_",
            ty: ": u8 = ",
            exempt: &[],
            min_members: 12,
            bits: ARCH_BITS,
            live_max: 11,
        },
        Domain {
            what: "deploy placement",
            sim_site: "deploy.rs PLACE_*",
            wire_site: "PLACEMENT_BITS",
            home: "deploy.rs",
            prefix: "pub const PLACE_",
            ty: ": u8 = ",
            exempt: &[],
            min_members: 5,
            bits: PLACEMENT_BITS,
            live_max: 4,
        },
        Domain {
            what: "craft station",
            sim_site: "craft.rs STATION_*",
            wire_site: "STATION_BITS",
            home: "craft.rs",
            prefix: "pub const STATION_",
            ty: ": u8 = ",
            exempt: &["MAX"],
            min_members: 5,
            bits: STATION_BITS,
            live_max: 4,
        },
        Domain {
            what: "lock grant",
            sim_site: "lock.rs GRANT_*",
            wire_site: "LOCK_GRANT_BITS",
            home: "lock.rs",
            prefix: "pub const GRANT_",
            ty: ": u8 = ",
            exempt: &[],
            min_members: 3,
            bits: LOCK_GRANT_BITS,
            live_max: 2,
        },
        Domain {
            what: "impact surface",
            sim_site: "ranged.rs SURF_*",
            wire_site: "SURF_BITS",
            home: "ranged.rs",
            prefix: "pub const SURF_",
            ty: ": u8 = ",
            exempt: &[],
            min_members: 3,
            bits: SURF_BITS,
            // A fourth kind fits the two bits, which is exactly why this
            // pin is not the same claim as the fit check: `SURF_BITS`'
            // spare value is currently *refused* at both ends, and a
            // widening that quietly made it live would turn a forged byte
            // into a fact. Moving this is a deliberate act, once per kind.
            live_max: 2,
        },
        Domain {
            what: "access op",
            sim_site: "deploy.rs ACCESS_OP_*",
            wire_site: "ACCESS_OP_BITS",
            home: "deploy.rs",
            prefix: "pub const ACCESS_OP_",
            ty: ": u8 = ",
            exempt: &["MAX"],
            min_members: 9,
            bits: ACCESS_OP_BITS,
            live_max: 8,
        },
        Domain {
            what: "bag-removal reason",
            sim_site: "backpack.rs BAG_GONE_*",
            wire_site: "BAG_GONE_BITS",
            home: "backpack.rs",
            prefix: "pub const BAG_GONE_",
            ty: ": u32 = ",
            // The ledger's name, not a member — the consume-refusal row's
            // note in full.
            exempt: &["MAX"],
            min_members: 3,
            bits: BAG_GONE_BITS,
            live_max: 2,
        },
    ];

    /// Scrape one domain's live members out of **every** module.
    ///
    /// Crate-wide, not per-file, and that is the whole point: a domain is
    /// a set of values the sim can emit, and the sim is the crate. Which
    /// file a member sits in is a fact about tidiness; whether the value
    /// exists at all is a fact about the wire. Reading `d.home` alone
    /// answered the second question with the first one's evidence.
    ///
    /// Deliberately line-oriented and not block-terminated: four of the
    /// ten families interleave doc comments between members, so a scan
    /// that stops at the first non-`const` line reads two of `world.rs`'s
    /// three causes and calls the domain covered.
    fn members(d: &Domain) -> Vec<(&'static str, u32, &'static str)> {
        let mut found = Vec::new();
        for m in SOURCES {
            for line in m.src.lines() {
                let Some(rest) = line.trim().strip_prefix(d.prefix) else {
                    continue;
                };
                let Some((name, value)) = rest.split_once(d.ty) else {
                    continue;
                };
                if d.exempt.contains(&name) {
                    continue;
                }
                let value = value.trim_end_matches(';');
                let v: u32 = value.parse().unwrap_or_else(|_| {
                    panic!(
                        "{}: {}{} in {} is declared as `{value}`, which is \
                         not a literal. This gate reads the constant block \
                         as text to learn the domain's range; a non-literal \
                         member makes that range unknowable, so either give \
                         it a literal or add it to `exempt` because it names \
                         the ledger rather than sitting in it.",
                        d.what, d.prefix, name, m.file
                    )
                });
                found.push((name, v, m.file));
            }
        }
        found
    }

    /// Every sim-side domain still fits the wire field that carries it.
    ///
    /// This is the assertion that would have failed the 2026-08-05 FAIL at
    /// `cargo test` instead of at a player's death screen.
    #[test]
    fn every_domain_fits_its_wire_field() {
        for d in DOMAINS {
            let found = members(d);

            assert!(
                found.len() as u32 >= d.min_members,
                "{}: only {} members parsed out of {} — the constant \
                 block's shape changed and this gate is now reading \
                 nothing, which is worse than failing. Fix the scrape \
                 before trusting the green.",
                d.what,
                found.len(),
                d.sim_site
            );

            // A domain lives in exactly one module. The scrape above is
            // crate-wide, so a stray member is already counted in
            // `highest` and will trip the pins below — but it would trip
            // them with a number and no address. This says the file.
            for (name, v, file) in &found {
                assert_eq!(
                    *file, d.home,
                    "{}: {}{} = {v} is declared in {file}, and this domain's \
                     home is {}. The value is on the wire either way — the \
                     scrape reads the whole crate precisely because the \
                     2026-08-05 failure was a death cause added one module \
                     away from the block that bounds it. Move it to {}, or \
                     move the domain's home here and say which in the same \
                     commit.",
                    d.what, d.prefix, name, d.home, d.home
                );
            }

            let highest = found.iter().map(|(_, v, _)| *v).max().unwrap();
            let capacity = 1u32 << d.bits;
            assert!(
                highest < capacity,
                "{}: {} declares a value {highest}, and {} is {} bits — \
                 which holds 0..={}. The encoder would refuse every event \
                 carrying it with Err(Range), the server would count the \
                 error and send nothing, and the client would never learn \
                 the fact happened. Widen the field, bump PROTO_VER and \
                 regenerate the goldens in this same commit (CLAUDE.md \
                 wall 6) — a widened meaning is a wire change even when no \
                 layout moves (lib.rs).",
                d.what,
                d.sim_site,
                d.wire_site,
                d.bits,
                capacity - 1
            );

            // And the domain has not been widened into spare headroom.
            // This is the check the fit test above cannot make: a fourth
            // death cause fits two bits, so every numeric wall stays green
            // while a pattern both ends refused as forged becomes a live
            // fact. That is a wire change with no layout change, which is
            // the one thing wall 6's byte-golden is structurally blind to.
            assert_eq!(
                highest, d.live_max,
                "{}: {} now tops out at {highest}, pinned here as {}. If \
                 that is deliberate it is still a wire change — a value \
                 both ends used to refuse is now meaningful, so an old \
                 client and a new server disagree about a packet whose \
                 bytes are identical. Bump PROTO_VER, regenerate the \
                 goldens and move this pin, all in this same commit \
                 (CLAUDE.md wall 6; lib.rs on widened meanings).",
                d.what, d.sim_site, d.live_max
            );
        }
    }

    /// The table itself must stay honest about which domains exist.
    ///
    /// Ten is not a count of convenience: it is every private `*_BITS` in
    /// this module that bounds a `sim-core` enumeration rather than a
    /// length, an index or a quantity. Widths like `INV_COUNT_BITS` or
    /// `NAME_LEN_BITS` bound a *magnitude* the sim computes and are not
    /// domains — they have no constant block to drift against.
    ///
    /// Note what this does **not** do: it fires when the table changes
    /// size, so it catches a row removed or added, and it cannot catch a
    /// width that never asked for a row. `every_enumeration_width_is_classified`
    /// is the gate for that, and it is the stronger of the two — this one
    /// is kept because a pinned count states the ledger's size as a fact
    /// somebody chose, which a scrape derives and therefore cannot assert.
    #[test]
    fn the_domain_table_states_its_own_coverage() {
        assert_eq!(
            DOMAINS.len(),
            16,
            "the wire-domain table changed size. Every entry is a field \
             width spent on a sim-core enumeration; add the new pair here \
             in the same commit that adds the width, or state why the \
             width bounds a magnitude rather than a domain."
        );
    }

    /// `SOURCES` is the whole crate, checked against `lib.rs`'s own list.
    ///
    /// The crate-wide scrape is only as wide as this table, and
    /// `include_str!` takes a literal path, so the table is hand-written
    /// and a new `sim-core` module is one `mod` line away from being
    /// invisible to every domain check above — the same hole again, one
    /// level up. `lib.rs` has to declare the module for it to compile, so
    /// that declaration is the thing to check against.
    #[test]
    fn the_source_table_covers_the_whole_crate() {
        const LIB: &str = include_str!("../../sim-core/src/lib.rs");

        let mut declared = Vec::new();
        for line in LIB.lines() {
            let t = line.trim();
            let Some(rest) = t
                .strip_prefix("pub mod ")
                .or_else(|| t.strip_prefix("mod "))
            else {
                continue;
            };
            // `mod foo;` only — an inline `mod tests {` is not a file.
            let Some(name) = rest.strip_suffix(';') else {
                continue;
            };
            declared.push(name);
        }

        assert!(
            declared.len() >= 20,
            "only {} `mod` declarations parsed out of sim-core/src/lib.rs — \
             the declaration shape changed and this gate is reading nothing, \
             which is worse than failing.",
            declared.len()
        );

        for name in &declared {
            assert!(
                SOURCES
                    .iter()
                    .any(|m| m.file.strip_suffix(".rs") == Some(name)),
                "sim-core declares `mod {name};` and SOURCES has no row for \
                 it, so every domain check in this module is blind to \
                 {name}.rs. A domain member declared there would pass all \
                 ten pins and still be refused by the encoder at runtime — \
                 that is the 2026-08-05 failure. Add the include_str! row."
            );
        }

        // And no stale rows: a deleted module leaves a path that still
        // compiles only until it does not, and a renamed one silently
        // narrows the scrape back to where it started.
        for m in SOURCES {
            let stem = m.file.strip_suffix(".rs").unwrap();
            assert!(
                stem == "lib" || declared.contains(&stem),
                "SOURCES lists {} and sim-core/src/lib.rs declares no \
                 `mod {stem};`. Either the module was renamed and the \
                 scrape is now reading a file nothing imports, or this row \
                 is stale — remove it or fix the name.",
                m.file
            );
        }
    }

    /// Every width this module spends is classified as a domain or a
    /// magnitude — by scraping the widths, not by counting the table.
    ///
    /// `the_domain_table_states_its_own_coverage` pins `DOMAINS.len()`,
    /// which fires only when the table *changes size*. It cannot make a
    /// newly added width acquire a row: spend `TOOL_TIER_BITS` on a new
    /// `sim-core` enumeration, bound it nowhere, and the count is still
    /// ten and still green. So this reads this file's own `*_BITS` block
    /// and forces each width into one of two named sets. A width that is
    /// in neither fails, and the fix is a sentence either way.
    #[test]
    fn every_enumeration_width_is_classified() {
        const WIRE_SRC: &str = include_str!("event.rs");

        /// Widths that bound a *magnitude* — a length, an index, a count,
        /// a duration — computed by the sim rather than chosen from a
        /// constant block. These have no domain to drift against, which
        /// is exactly why they are listed by name rather than by rule:
        /// the distinction is a judgement, and it should be made once,
        /// here, by whoever adds the width.
        const MAGNITUDES: &[&str] = &[
            "SUB_BITS",
            "MOVE_SLOT_BITS",
            "INV_COUNT_BITS",
            "INV_SLOT_BITS",
            "SYNC_COUNT_BITS",
            "CATALOG_TOTAL_BITS",
            "CATALOG_COUNT_BITS",
            "NAME_LEN_BITS",
            "CRAFT_Q_COUNT_BITS",
            "RECIPE_TOTAL_BITS",
            "RECIPE_COUNT_BITS",
            "RESEARCH_TOTAL_BITS",
            "RESEARCH_COUNT_BITS",
            "RECIPE_TICKS_BITS",
            "N_INPUTS_BITS",
            "PIECE_SYNC_COUNT_BITS",
            "PIECE_DEFS_TOTAL_BITS",
            "PIECE_DEFS_COUNT_BITS",
            "N_COSTS_BITS",
            "DEPLOY_COSTS_BITS",
            "DEPLOY_SYNC_COUNT_BITS",
            "DEPLOY_DEFS_TOTAL_BITS",
            "DEPLOY_DEFS_COUNT_BITS",
            "STOCK_COUNT_BITS",
            "BAG_SYNC_COUNT_BITS",
            // A count bounded by `BAG_CAP`, which is a sim-core *cap* and
            // not an enumeration — `INV_COUNT_BITS`' and
            // `STOCK_COUNT_BITS`' shape, so it is classified with them.
            // What guards it against the cap being raised is the
            // compile-time assert beside the declaration, which is the
            // guard a DOMAINS row would have bought and cheaper.
            "BAGS_COUNT_BITS",
            "CONT_COUNT_BITS",
            "LOCK_CODE_BITS",
        ];

        let mut widths = Vec::new();
        for line in WIRE_SRC.lines() {
            // An optional `pub(crate)` first: two of these widths are
            // spent by the C→S encoder in `lib.rs` and declared here so
            // this gate can see them at all. A scrape that only read
            // private consts would have skipped exactly the widths that
            // needed a reason to live in this file.
            let line = line.trim();
            let line = line.strip_prefix("pub(crate) ").unwrap_or(line);
            let Some(rest) = line.strip_prefix("const ") else {
                continue;
            };
            let Some((name, _)) = rest.split_once(": u32 = ") else {
                continue;
            };
            if !name.ends_with("_BITS") {
                continue;
            }
            widths.push(name);
        }

        assert!(
            widths.len() >= 33,
            "only {} `*_BITS` widths parsed out of event.rs — the \
             declaration shape changed and this gate is reading nothing.",
            widths.len()
        );

        for name in &widths {
            let is_domain = DOMAINS.iter().any(|d| d.wire_site == *name);
            let is_magnitude = MAGNITUDES.contains(name);
            assert!(
                is_domain != is_magnitude,
                "{name} is {}. Every width this module spends is one of \
                 two things: a *domain*, bounding a sim-core enumeration \
                 that can grow past it (add a DOMAINS row, and the ten \
                 pins there start guarding it), or a *magnitude*, bounding \
                 a length or index the sim computes (add it to MAGNITUDES \
                 above). A width in neither is the 2026-08-05 failure with \
                 nothing watching for it; a width in both is a table that \
                 contradicts itself.",
                if is_domain {
                    "both a DOMAINS row and a listed magnitude"
                } else {
                    "in neither DOMAINS nor MAGNITUDES"
                }
            );
        }

        // Every DOMAINS row names a width that actually exists — a typo
        // or a renamed const would otherwise leave a row guarding nothing
        // while still counting toward the ten.
        for d in DOMAINS {
            assert!(
                widths.contains(&d.wire_site),
                "{}: DOMAINS names its width `{}` and event.rs declares no \
                 such `const {}: u32`. The row is guarding a field that \
                 does not exist under that name.",
                d.what,
                d.wire_site,
                d.wire_site
            );
        }
    }

    /// The one domain whose bound is *derived* rather than restated stays
    /// derived, and the largest live value actually survives the encoder.
    ///
    /// The numeric checks above are necessary and not sufficient: they
    /// prove the value fits the field, not that the range check in
    /// `encode_event_death` agrees. This drives the real encoder with the
    /// highest cause the sim can produce and asserts bytes came out — the
    /// judge's own reproduction of the FAIL, kept as a gate rather than a
    /// report.
    #[test]
    fn the_highest_live_death_cause_encodes() {
        let mut buf = [0u8; MAX_EVENT_MSG_BYTES];
        for cause in 0..=sim_core::world::DEATH_BY_MAX {
            let n = encode_event_death(7, 9, cause, 3, 250, &mut buf).unwrap_or_else(|e| {
                panic!(
                    "death cause {cause} is live in the sim and the wire \
                     refused it ({e:?}). This is the 2026-08-05 failure \
                     exactly: the victim's client never learns it died, so \
                     the death screen never opens and the body is parked \
                     until it is killed again."
                )
            });
            match decode_event(&buf[..n]) {
                Ok(EventMsg::Death { cause: got, .. }) => assert_eq!(
                    got, cause,
                    "death cause {cause} did not survive the round trip"
                ),
                other => panic!("death cause {cause} decoded as {other:?}"),
            }
        }

        // And the first pattern past the domain is still refused at both
        // ends — the closed-set posture the domain comment claims.
        let forged = sim_core::world::DEATH_BY_MAX + 1;
        if (forged as u32) < (1 << DEATH_CAUSE_BITS) {
            assert_eq!(
                encode_event_death(7, 9, forged, 3, 250, &mut buf),
                Err(WireError::Range),
                "cause {forged} is not a live death and the encoder let it \
                 through — the domain is no longer closed"
            );
        }
    }

    /// The two non-saturating refusal domains refuse their slack at BOTH
    /// ends (NOW.md §5b, decode side). `BAG_GONE_*` tops out at 2 in a
    /// 2-bit field and `REFUSE_C_*` at 3 in a 4-bit one, so `why == 3` and
    /// `reason` 4..=15 fit the width while naming nothing in the sim's
    /// ledger. Until 2026-08-17 the decoder passed them through intact —
    /// a value no rule owns, handed to the HUD — and `encode_event_bag_
    /// removed` checked only the width. Both bounds now derive from the
    /// sim's own `*_MAX` (`the_highest_live_death_cause_encodes`' posture),
    /// and the forged patterns are `Malformed`, which the client's pump
    /// counts and drops (`ClientCore::on_stream`) — never a panic, never a
    /// disconnect. Forged by hand because the encoder refuses them.
    #[test]
    fn the_refusal_slack_refuses_at_both_ends() {
        let mut buf = [0u8; MAX_EVENT_MSG_BYTES];

        // Every live bag-removal reason still round-trips…
        for why in 0..=sim_core::backpack::BAG_GONE_MAX {
            let len = encode_event_bag_removed(11, why as u8, &mut buf).unwrap();
            match decode_event(&buf[..len]).unwrap() {
                EventMsg::BagRemoved { id: 11, why: got } => assert_eq!(got as u32, why),
                other => panic!("why {why} decoded as {other:?}"),
            }
        }
        // …the encoder refuses the first pattern past the ledger (it fits
        // the width, which is exactly why the check must be the domain)…
        let forged_why = sim_core::backpack::BAG_GONE_MAX + 1;
        const { assert!(sim_core::backpack::BAG_GONE_MAX + 1 < (1 << BAG_GONE_BITS)) };
        assert_eq!(
            encode_event_bag_removed(11, forged_why as u8, &mut buf),
            Err(WireError::Range),
            "why {forged_why} names no BAG_GONE_* and the encoder let it through"
        );
        // …and the decoder refuses it off the wire, because a client that
        // trusted the encoder's checks would be trusting a server it has
        // no reason to.
        let mut w = BitWriter::new(&mut buf);
        w.write(KIND_EVENT, KIND_BITS).unwrap();
        w.write(SUB_BAG_REMOVED, SUB_BITS).unwrap();
        w.write(11, 32).unwrap();
        w.write(forged_why, BAG_GONE_BITS).unwrap();
        let len = w.finish();
        assert_eq!(
            decode_event(&buf[..len]),
            Err(WireError::Malformed),
            "a forged why == {forged_why} must be refused, not passed through"
        );

        // The consume refusal: every value the width holds past the
        // ledger — the whole 4..=15 forgery range — is `Malformed`.
        for reason in (sim_core::survival::REFUSE_C_MAX + 1)..(1 << REFUSE_C_BITS) {
            let mut w = BitWriter::new(&mut buf);
            w.write(KIND_EVENT, KIND_BITS).unwrap();
            w.write(SUB_CONSUME_REFUSED, SUB_BITS).unwrap();
            w.write(reason, REFUSE_C_BITS).unwrap();
            let len = w.finish();
            assert_eq!(
                decode_event(&buf[..len]),
                Err(WireError::Malformed),
                "reason {reason} names no REFUSE_C_* and decoded anyway"
            );
        }
        // The encoder half already held (`encode_event_consume_refused`);
        // pinned here so the two ends are asserted side by side.
        assert_eq!(
            encode_event_consume_refused((sim_core::survival::REFUSE_C_MAX + 1) as u8, &mut buf),
            Err(WireError::Range)
        );
        // And the live set still crosses.
        for reason in 1..=sim_core::survival::REFUSE_C_MAX {
            let len = encode_event_consume_refused(reason as u8, &mut buf).unwrap();
            assert_eq!(
                decode_event(&buf[..len]).unwrap(),
                EventMsg::ConsumeRefused {
                    reason: reason as u8
                }
            );
        }
    }

    /// **Every encoder must have a decoder**, checked against this file's
    /// own source rather than against a list somebody maintains.
    ///
    /// `SUB_KNOWN` shipped at research v0 with `encode_event_known`, a
    /// `EventMsg::Known` variant, a server that called it on every
    /// purchase and a `ClientCore` handler waiting for it — and no arm in
    /// `decode_event`. Every one of those frames came back `Malformed`, so
    /// the client's blueprint mask was never set by anything, for the whole
    /// life of the feature. Nothing caught it. `test_protocol_golden` pins
    /// what the encoder emits and an encoder with no reader is byte-perfect;
    /// the round-trip tests above cover the subtypes somebody remembered to
    /// write a round trip for, which is exactly the set that was never the
    /// problem.
    ///
    /// So the check is structural. Every `begin(buf, SUB_X)` in this file
    /// names a subtype the encoder can produce; every `SUB_X =>` names one
    /// the decoder can read. The first set must be a subset of the second,
    /// and a half-built subtype is red the moment its encoder lands.
    /// A bare Rust identifier — letters, digits and underscores, nothing
    /// else. What separates a real `SUB_*` constant from a fragment of a
    /// string literal or a grouped match pattern.
    fn is_ident(s: &str) -> bool {
        !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    }

    #[test]
    fn every_encoder_has_a_decoder() {
        const SRC: &str = include_str!("event.rs");

        let mut encoded: Vec<&str> = Vec::new();
        let mut decoded: Vec<&str> = Vec::new();
        for line in SRC.lines() {
            let line = line.trim();
            // Comments are not code, and this gate's own prose names both
            // shapes it scans for — it found itself on the first run.
            if line.starts_with("//") {
                continue;
            }
            // The encoder side: `let mut w = begin(buf, SUB_X)?;`
            if let Some(i) = line.find("begin(buf, SUB_") {
                let rest = &line[i + "begin(buf, ".len()..];
                if let Some((name, _)) = rest.split_once(')') {
                    // An identifier only. This gate's own source contains
                    // the string literal it scans for, so the second thing
                    // it found was a fragment of itself — `SUB_"`.
                    if is_ident(name) {
                        encoded.push(name);
                    }
                }
            }
            // The decoder side: `SUB_X => …`, the match arms.
            if line.starts_with("SUB_") {
                if let Some((name, _)) = line.split_once(" =>") {
                    if is_ident(name) {
                        decoded.push(name);
                    }
                }
            }
        }

        assert!(
            encoded.len() > 20,
            "the encoder scan found only {} subtypes — the `begin(buf, …)` \
             shape changed and this gate is now checking nothing",
            encoded.len()
        );
        assert!(
            decoded.len() > 20,
            "the decoder scan found only {} arms — the match's shape \
             changed and this gate is now checking nothing",
            decoded.len()
        );

        for name in &encoded {
            assert!(
                decoded.contains(name),
                "{name} has an encoder and no arm in `decode_event`, so \
                 every frame it produces decodes as `Malformed` and the \
                 fact it carries never reaches a client. This is exactly \
                 how SUB_KNOWN shipped dead. Add the arm in the same \
                 commit as the encoder."
            );
        }
    }

    /// And the mask itself round-trips, both halves, through the real
    /// encoder and the real decoder — the check whose absence let the
    /// missing arm hide. A mask with only low bits set would pass with the
    /// two halves swapped.
    #[test]
    fn known_decodes_to_the_mask_that_was_encoded() {
        let mut buf = [0u8; MAX_EVENT_MSG_BYTES];
        for mask in [0u64, 1, 1 << 3 | 1 << 40, 1 << 63, u64::MAX] {
            let len = encode_event_known(mask, &mut buf).unwrap();
            assert_eq!(
                decode_event(&buf[..len]).unwrap(),
                EventMsg::Known { mask },
                "the blueprint mask {mask:#x} did not survive the wire"
            );
        }
    }
}
