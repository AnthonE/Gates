//! Deployables + the hearth (DESIGN.md §2, M1): placeable entities baked
//! from `content/deployables.toml`, the hearth's building-privilege claim,
//! and the upkeep/decay sweep that makes the map heal itself. Pure and
//! fixed-capacity like the other verbs: content reaches the sim only as
//! the baked `DeployContent` table, the inert `EMPTY` default makes every
//! request refuse, and `probe_fixture()` is the synthetic table for the
//! parity/replay/alloc gates.
//!
//! Model v0 (proposed defaults, DECISIONS.md §open "deployables v0" and
//! "upkeep/decay v0"):
//! - A deployable occupies one grid address — body deploys at
//!   (cell, level, LOC_PLANE), doors at a doorway's edge address. Placing
//!   consumes one unit of the deployable's own item.
//! - Placement class (schema order): `ground` = level 0 on buildable
//!   terrain with no plane piece; `foundation` = on a plane piece at the
//!   level; `doorway` = in a doorway edge piece; `any` = foundation, else
//!   ground.
//! - A **hearth** claims building privilege over its base's own volume
//!   (`claim.rs`, privilege v1): piece and deployable placement inside a
//!   *foreign* claim refuses (`REFUSE_B_CLAIM` / `REFUSE_D_CLAIM`). No
//!   hearth may be placed inside any claim, own included.
//! - **Upkeep**: every `UPKEEP_PERIOD_TICKS`, each placed piece charges
//!   `ceil(cost × upkeep_pct_per_day / 100 / 24)` per cost row from the
//!   first hearth (list order) whose claim volume covers it and can pay
//!   that row — the **same shape the build verbs answer to**, read from
//!   `claim::ClaimCache` because the sweep runs per tick where the walk
//!   runs per keypress. Unpaid rows **decay** the piece by its
//!   material's ladder rate (flat `DECAY_PCT_PER_PERIOD` when no ladder
//!   is priced); at 0 hp the piece is removed, and any deployable at the
//!   removed piece's exact address goes with it.
//! - Deployables charge nothing: covered by any hearth with stock, they
//!   are free; uncovered, they decay on the same cadence (the map sheds
//!   its litter). An empty hearth does not cover itself.
//! - The **feed** action moves up to `FEED_CHUNK` units per upkeep
//!   material per press from the feeder's inventory into a hearth in
//!   reach (any hearth — feeding is a gift, only placement is owner-
//!   gated), capped at `STOCK_MAX` per material. Withdrawal waits for the
//!   container UI; stock readback rides the feed ack only.
//!
//! - **Doors** (door v0, DECISIONS.md §open): a door places **closed**
//!   and seals its doorway (collide.rs shut bits — the store keeps them
//!   in lockstep exactly like the piece store keeps its masks). The
//!   **use** action toggles it open/closed for any player within build
//!   reach — subject to the lock below. EV_DOOR announces the new state
//!   for the wire; removal (decay, or the doorway decaying under it)
//!   clears the shut bit with the record. The state is absolute on the
//!   wire, never a delta, so the client that toggled optimistically
//!   (NETCODE.md §6.1) is confirmed or corrected by the same event.
//! - **Locks** (lock v1, DECISIONS.md §open, `reference/DOORS.md` §9): a
//!   door places **bare** — no lock, workable by anyone in reach — and
//!   security is a **code lock**, an item somebody crafted and bolted on
//!   (`ARCH_LOCK`, placement class `door`). The lock and everything it
//!   remembers live in `lock.rs`'s dense store, keyed by the door's own
//!   address exactly as a hearth's stock and a box's contents are keyed by
//!   theirs; `DeployRec::locked` survives as a **mirror** of that store's
//!   verdict, which is what keeps the wire, the client and EV_DOOR
//!   unchanged from lock v0. The **lock** action carries an op and a
//!   four-digit code (`deploy::ACCESS_OP_*`) and is absolute rather than a
//!   toggle, for lock v0's reason: two presses racing must agree on the
//!   result, not swap it. Locking never moves the leaf.
//!
//!   A **box takes the same lock** (`DOORS.md` §9.8): [`lockable`] names
//!   the two archetypes, the box's lid asks `Locks::passes` where the
//!   door's leaf does (`world.rs`'s move verb, the box's only open path),
//!   and the refusal is the door's own `REFUSE_D_OWNER` — *this lock does
//!   not know you* is one sentence whichever thing refused it.
//!
//!   This retires lock v0's "a door places locked to its placer". That
//!   rule made the door free and the security free; the reference makes
//!   the door free and charges for the security, and an unlocked door
//!   stops being a state a player chooses and becomes the state of a door
//!   nobody has paid for yet (`DOORS.md` §9.2).
//!
//! Not in this slice (documented, not forgotten): no support cascade when
//! a piece decays away (floating pieces keep decaying on their own if
//! unpaid), and no **key** lock, which is blocked on per-item instance
//! data our `ItemStack` has no room for and is the system the reference
//! itself gave up on (§9.7).

use crate::build::{
    BuildContent, Pieces, LEVEL_H_M, LOC_EDGE_XLO, LOC_EDGE_ZLO, LOC_PLANE, SHAPE_DOORWAY,
};
use crate::craft::{inv_count, inv_take};
use crate::gather::{GatherContent, ItemStack};
use crate::limits::{
    BOX_SLOTS, HEARTH_CREW_CAP, HEARTH_STOCK_ROWS, INV_SLOTS, MAX_BOXES, MAX_BOX_SPILL_PER_TICK,
    MAX_BUILD_COORD, MAX_BUILD_LEVELS, MAX_DEPLOYS, MAX_DEPLOY_COSTS, MAX_DEPLOY_DEFS, MAX_HEARTHS,
    MAX_LOCKS, UPKEEP_SWEEP_PER_TICK,
};
use crate::lock::{self, LockRec, Locks, Outcome};
use crate::roster::{Added, Roster};
use crate::terrain;
use crate::world::{
    EventQueue, Player, EV_AUTH, EV_DEPLOY_PLACED, EV_DEPLOY_REFUSED, EV_DEPLOY_REMOVED, EV_DOOR,
    EV_HEALTH, EV_KNOCK, EV_PIECE_REMOVED, EV_STOCK, EV_STRUCT_HIT, STRUCT_DEPLOY_BIT,
};

/// Archetype codes (schema order: CONTENT.md §1 deployable).
pub const ARCH_BAG: u8 = 0;
pub const ARCH_HEARTH: u8 = 1;
pub const ARCH_BOX: u8 = 2;
pub const ARCH_FIRE: u8 = 3;
pub const ARCH_FURNACE: u8 = 4;
pub const ARCH_WORKBENCH: u8 = 5;
pub const ARCH_DOOR: u8 = 6;
/// A code lock (lock v1). The one archetype that does **not** become a
/// `DeployRec`: it bolts onto a door's address and lives in `lock.rs`'s
/// store, because the reference's lock is a separate entity parented to
/// the door and ours has to be a separate record for the same reason —
/// `DeployRec` is what the wire mirrors, and a code, two lists and two
/// timers may not ride on it.
pub const ARCH_LOCK: u8 = 7;
/// The recycler (recycler v0) — a container that converts without burning.
/// It is an oven in every way but the fuel: `oven.rs` holds the state, the
/// same sweep advances it, and what separates it from a furnace is which
/// `cooking.toml` rows name it. The one mechanism it does not share is the
/// burn — a recycler lights on a press and spends nothing to run.
///
/// **The ninth archetype, and that was not free.** Three bits held exactly
/// eight, so this const cost `ARCH_BITS` a widening to four and `PROTO_VER`
/// a turn with every golden regenerated in the same commit (wall 6). The
/// gate that says so is `every_domain_fits_its_wire_field`, and it is why
/// the tenth archetype is cheap and the seventeenth is not.
pub const ARCH_RECYCLER: u8 = 8;
/// A research table (research v0). A **station**, not a container: it is
/// checked by proximity the way `craft.rs` checks a workbench, so it holds
/// no items, mints no box record, and the thing being researched comes out
/// of the player's own inventory (`research.rs` says why).
///
/// The tenth archetype, and free — `ARCH_BITS` widened to four for the
/// recycler one commit earlier and holds sixteen. That is what the
/// recycler's price bought.
pub const ARCH_RESEARCH: u8 = 9;
/// Workbench level 2 — the bench ladder's second rung (bench ladder v0,
/// the pre-Oct-2025 scrap-era shape, operator 2026-08-15). Its own
/// archetype rather than a tier field on the def, for the same reason the
/// recycler is not a furnace with a flag: the archetype is what the wire
/// carries, what the client draws a silhouette from, and what
/// `bench_near` scans — one ledger, three readers. A higher bench
/// satisfies a lower recipe (`bench_tier`), so placing this retires
/// nothing.
pub const ARCH_WORKBENCH2: u8 = 10;
/// Workbench level 3 — the top rung, the gate a raid kit stands behind.
pub const ARCH_WORKBENCH3: u8 = 11;

/// The blocked volume of each archetype, `[w, h, d]` full extents in
/// metres, centred on the deploy's cell centre with its base at the
/// piece formula's height (`collide::col_base_y + level·LEVEL_H_M` — the
/// same expression the client's `level_base_y` draws at). **A zero row
/// never blocks**, and four rows are zero on purpose:
///
/// - **bag** — a 0.32 m mat a body walks over, and the respawn anchor; a
///   bag that blocked would let eight of them wall a doorway for cloth.
/// - **fire** — the genre walks over its campfire, and the burn the pit
///   owes a body standing in it is a damage feature, not a wall.
/// - **door** — blocks as an *edge*, through the shut bits the store
///   already keeps in lockstep (`collide::ColMasks::shut_*`).
/// - **lock** — never a `DeployRec` at all.
///
/// The six non-zero rows are the client's own authored `DEPLOY` sizes
/// (`render/structures.rs`), digit for digit — real-world dimensions,
/// `DECISIONS.md` §open "deployable proportions" — and
/// `crates/client/tests/greybox.rs` holds the two tables equal, which is
/// the comparison that item said could not exist while the sim had no
/// table. Sim truth now: a row here is a collision change (wall 5 —
/// `test_replay` moves with it, deliberately).
pub const DEPLOY_VOL: [[f32; 3]; 12] = [
    [0.0, 0.0, 0.0],   // 0 bag — walk-over
    [1.2, 1.0, 0.6],   // 1 hearth
    [1.2, 0.65, 0.7],  // 2 box
    [0.0, 0.0, 0.0],   // 3 fire — walk-over
    [1.3, 0.95, 0.85], // 4 furnace
    [1.6, 0.9, 0.7],   // 5 workbench
    [0.0, 0.0, 0.0],   // 6 door — the shut bit's business
    [0.0, 0.0, 0.0],   // 7 lock — never a record
    [1.3, 1.15, 0.9],  // 8 recycler
    [1.5, 0.8, 0.8],   // 9 research table
    [1.6, 1.0, 0.8],   // 10 workbench 2 — taller and deeper than 5
    [1.8, 1.1, 0.8],   // 11 workbench 3 — the widest bench
];

/// The blocked volume of `arch` as `(half_w, h, half_d)`, or `None` for
/// an archetype that never blocks. The one lookup both `collide.rs`'s
/// queries and the lockstep writers below go through.
#[inline]
pub fn solid_vol(arch: u8) -> Option<(f32, f32, f32)> {
    let [w, h, d] = *DEPLOY_VOL.get(arch as usize)?;
    if h <= 0.0 {
        return None;
    }
    Some((w * 0.5, h, d * 0.5))
}

const _: () = {
    // Every archetype code must fit the collision index's 4-bit nibble
    // (`collide::ColMasks::solid`), with 0xF left over as its empty
    // sentinel.
    assert!(ARCH_WORKBENCH3 < 0xF);
    // A solid deploy stands at its cell centre, so the movement query
    // tests only the candidate's own build cell. That is complete iff no
    // volume, inflated by the capsule, can reach past the half-cell:
    // max(w, d)/2 + CAPSULE_RADIUS_M < BUILD_CELL_M/2. Checked here for
    // every row so a fatter row cannot land without re-proving the reach.
    let mut i = 0;
    while i < DEPLOY_VOL.len() {
        let w = DEPLOY_VOL[i][0];
        let d = DEPLOY_VOL[i][2];
        let half = if w > d { w * 0.5 } else { d * 0.5 };
        assert!(half + crate::collide::CAPSULE_RADIUS_M < crate::build::BUILD_CELL_M * 0.5);
        i += 1;
    }
};

/// The **access verb's** operations — `Command::Access`'s `op`, wire
/// `ACT_ACCESS`. One action with an op field rather than nine action
/// codes, because the action space was full at 15 in four bits and a
/// width bump there costs every C→S message a bit; the op is four bits
/// inside this one payload instead.
///
/// **One op space, two stores, and that is the point.** 0..=5 run against
/// the code lock at a door (`lock.rs`); 6..=8 run against the hearth's
/// crew (`crew_op`). They share a space because they are one question —
/// *who may do this here* — and the answer is a `Roster` either way; and
/// they share a *wire field*, so a second space would mean a second width
/// for the domain gate to bound and a second chance to get it wrong.
///
/// The home of the whole space is here rather than in `lock.rs` because
/// the dispatcher is here: a module that declared ops it does not
/// implement would be a comment that goes stale silently.
pub const ACCESS_OP_SET_CODE: u8 = 0;
pub const ACCESS_OP_SET_GUEST: u8 = 1;
pub const ACCESS_OP_ENTER: u8 = 2;
pub const ACCESS_OP_LOCK: u8 = 3;
pub const ACCESS_OP_UNLOCK: u8 = 4;
pub const ACCESS_OP_TAKE: u8 = 5;
/// Join the crew of the hearth at the address (hearth crew v1).
pub const ACCESS_OP_CREW_JOIN: u8 = 6;
/// Leave it.
pub const ACCESS_OP_CREW_LEAVE: u8 = 7;
/// Clear it back to the clearer alone.
pub const ACCESS_OP_CREW_CLEAR: u8 = 8;
/// The highest op above, named rather than counted — the wire refuses
/// past it and the sim refuses past it, in that order.
pub const ACCESS_OP_MAX: u8 = ACCESS_OP_CREW_CLEAR;
/// Whether an op addresses the hearth's crew rather than a door's lock.
/// The one place the split is written down, so the wire's range check and
/// the dispatcher cannot disagree about which half an op belongs to.
pub fn op_is_crew(op: u8) -> bool {
    matches!(
        op,
        ACCESS_OP_CREW_JOIN | ACCESS_OP_CREW_LEAVE | ACCESS_OP_CREW_CLEAR
    )
}

/// Does a deployable of this archetype hold items — that is, does
/// placing it stand up a container record in the box store?
///
/// Four archetypes say yes and they share one store deliberately: a
/// storage box is a container, an oven (`oven.rs`) is a container that
/// burns, and a recycler is a container that converts. One store means
/// one address space, one `CONT_BOX` handle, one reach rule and one spill
/// path when a raid takes the thing apart — so a campfire's contents fall
/// on the floor by the same route a box's do, with no second contract on
/// the wire and none in the sim.
pub fn holds_items(arch: u8) -> bool {
    arch == ARCH_BOX || arch == ARCH_FIRE || arch == ARCH_FURNACE || arch == ARCH_RECYCLER
}

/// A workbench archetype's rung on the bench ladder, `0` for anything
/// that is not a bench. The one place the arch→tier mapping is written,
/// so the craft gate (`craft::enqueue`), the tree gate
/// (`research::unlock`) and the scan above cannot disagree about what a
/// bench is worth. The tier deliberately equals the `STATION_WORKBENCH*`
/// code it satisfies (`craft.rs` asserts the two ladders line up).
#[inline]
pub const fn bench_tier(arch: u8) -> u8 {
    match arch {
        ARCH_WORKBENCH => 1,
        ARCH_WORKBENCH2 => 2,
        ARCH_WORKBENCH3 => 3,
        _ => 0,
    }
}

/// What a code lock may bolt onto: a door's leaf or a storage box's lid
/// (lock v1; locks on boxes, `reference/DOORS.md` §9.8). The fire and the
/// furnace are containers too and deliberately **not** here — the
/// reference locks neither, and an oven is a shared amenity whose cook
/// loop rewrites its own slots, so a lock on one would be a claim the
/// sim itself ignores every burn tick.
pub fn lockable(arch: u8) -> bool {
    arch == ARCH_DOOR || arch == ARCH_BOX
}

/// The one archetype the use verb toggles. Named so `use_door` and
/// `lock_op` can share `door_in_reach` while disagreeing about what may
/// stand at the address: a box carries a lock, but pressing E at one is
/// the container panel's business, never a leaf toggle.
fn arch_is_door(arch: u8) -> bool {
    arch == ARCH_DOOR
}

/// Placement-class codes (schema order).
pub const PLACE_GROUND: u8 = 0;
pub const PLACE_FOUNDATION: u8 = 1;
pub const PLACE_DOORWAY: u8 = 2;
pub const PLACE_ANY: u8 = 3;
/// On a door — the only class that wants the address **occupied**, and
/// occupied by one specific archetype at that.
pub const PLACE_DOOR: u8 = 4;

/// Integer refusal reasons (CLAUDE.md wall 3), carried by
/// EV_DEPLOY_REFUSED / the deploy-refused wire subtype.
pub const REFUSE_D_KIND: u32 = 0;
pub const REFUSE_D_SPOT: u32 = 1;
pub const REFUSE_D_SUPPORT: u32 = 2;
pub const REFUSE_D_TERRAIN: u32 = 3;
pub const REFUSE_D_REACH: u32 = 4;
pub const REFUSE_D_COST: u32 = 5;
pub const REFUSE_D_FULL: u32 = 6;
pub const REFUSE_D_CLAIM: u32 = 7;
pub const REFUSE_D_OVERLAP: u32 = 8;
pub const REFUSE_D_BAG_CAP: u32 = 9;
pub const REFUSE_D_HEARTH: u32 = 10;
/// A use request named an address holding no door — or a lock op named
/// one holding nothing a lock bolts to (`lockable`).
pub const REFUSE_D_DOOR: u32 = 11;
/// The lock says no: a use on a locked door you are not remembered by, a
/// move against a locked box's slots (locks on boxes, `world.rs`), or a
/// full-rights lock op from a guest or a stranger (lock v1). One reason
/// for every half, because all of them are the same sentence — *this lock
/// does not know you* — and a client that could tell them apart would
/// learn something about the lock it was refused by.
pub const REFUSE_D_OWNER: u32 = 12;
/// A use press on an oven with nothing in it that burns (oven v0,
/// `oven.rs`). Lighting is a match, not a delivery: the fuel has to be
/// inside already.
pub const REFUSE_D_FUEL: u32 = 13;
/// A lock op named an address carrying no lock (lock v1).
pub const REFUSE_D_NO_LOCK: u32 = 14;
/// A lock placement named a door that already carries one. The reference's
/// rule and ours: one lock per door, and bolting a second on is how a
/// raider would otherwise evict an owner for the price of an item.
pub const REFUSE_D_HAS_LOCK: u32 = 15;
/// Wrong code. The shock has already been taken off the sender's hp —
/// this is the announcement, not the punishment.
pub const REFUSE_D_CODE: u32 = 16;
/// The keypad is shut: `lock::LOCKOUT_TRIES` wrong codes inside
/// `lock::LOCKOUT_TICKS`. A *correct* code is refused too while it lasts,
/// which is the point.
pub const REFUSE_D_LOCKOUT: u32 = 17;
/// A remembered list is full (`limits.rs` `LOCK_AUTH_CAP` /
/// `LOCK_GUEST_CAP`). Wall 4's overflow policy, said out loud: refuse,
/// never evict — a door that forgot its owner is the one failure this cap
/// may not have.
pub const REFUSE_D_AUTH_FULL: u32 = 18;
/// A pickup named a container that still has something in it. Refused
/// rather than spilled: a box that vanished with its contents into an
/// inventory that could not hold them would be the worst kind of loss —
/// silent, and caused by the player's own verb.
pub const REFUSE_D_NOT_EMPTY: u32 = 19;
// Hearth crew v1 adds **no reason codes**, and that is deliberate. A crew
// op at an address holding no hearth is `REFUSE_D_HEARTH`, which the feed
// verb already says in those words; a crew op from a hand the hearth does
// not know is `REFUSE_D_OWNER`, for the reason the lock's is — *this
// thing does not know you* is one sentence whichever list refused it. The
// reason ledger is a closed set the client's table mirrors row for row
// (`client/ui/refusals.rs`), and two codes for one sentence is exactly
// how the two drift.

/// The **legacy** hearth privilege radius in meters, planar from the
/// hearth's cell center. Proposed default, DECISIONS.md §open
/// ("deployables v0"). No live path asks it any more: the build verbs
/// moved to the base's own volume with privilege v1 (`claim.rs`), and
/// the upkeep sweep followed onto the cached form of the same shape —
/// the split `NOW.md` §0aa item 1 named is closed. What still reads it
/// is [`Deploys::foreign_claim`], the circle kept as the crew tests'
/// probe.
pub const HEARTH_RADIUS_M: f32 = 24.0;
/// Upkeep/decay cadence: one period per real hour at the 30 Hz tick.
/// Proposed default, DECISIONS.md §open ("upkeep/decay v0").
pub const UPKEEP_PERIOD_TICKS: u64 = 108_000;
/// Periods per day — the divisor that spreads `upkeep_pct_per_day`
/// (content/balance.toml) over hourly charges. Calendar arithmetic, not
/// a knob.
pub const PERIODS_PER_DAY: u32 = 24;
/// Materials the decay ladder is keyed by — `build::MAT_*`, whose set is
/// closed and four long (twig, wood, stone, metal).
pub const DECAY_MATERIALS: usize = 4;

/// Fallback decay per period, % of max hp, for content that prices no
/// ladder and for the one store that has no material: **deployables**.
/// A box is not wood or stone in the piece sense, so it keeps the flat
/// rate the whole world used before upkeep/decay v1.
/// Proposed default, DECISIONS.md §open ("upkeep/decay v0").
pub const DECAY_PCT_PER_PERIOD: u32 = 5;
/// Units per upkeep material one feed press moves. Proposed default,
/// DECISIONS.md §open ("deployables v0").
pub const FEED_CHUNK: u32 = 100;
/// Stock ceiling per material per hearth. Proposed default, DECISIONS.md
/// §open ("deployables v0").
pub const STOCK_MAX: u32 = 2_000;
/// Bags one player may have placed (ALPHA.md §1 knob, DECISIONS.md §open
/// "bag cooldown · cap": 8).
pub const BAG_CAP: usize = 8;
/// Ticks a bag sleeps for after a body wakes on it — the other half of the
/// same spoken knob (ALPHA.md §1, DECISIONS.md §open "bag cooldown · cap":
/// **5 min**), in the only unit the sim owns. 5 × 60 × `TICK_HZ` = 9 000 at
/// the 30 Hz tick, written as the literal it evaluates to because the knob
/// registry gate pins declarations, not expressions;
/// `the_cooldown_is_five_minutes_of_ticks` is what holds the arithmetic.
pub const BAG_COOLDOWN_TICKS: u64 = 9_000;
/// Hour-steps one sweep visit processes at most (bounded catch-up; the
/// sweep revisits every entry far inside one period, so >1 only happens
/// after a tick-jump). Bounded-work constant, not a knob.
const SWEEP_CATCHUP_MAX: u32 = 4;

/// One baked deployable row. `hp == 0` ⇒ inert (the empty-table row).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeployDef {
    pub arch: u8,
    pub placement: u8,
    pub hp: u16,
    /// Item index placing consumes one unit of (the deployable's item).
    pub item: u16,
    /// Live rows of `costs`. Zero ⇒ the bake found no recipe for `item`,
    /// so this row has no quoted price and `repair` refuses rather than
    /// mending it free.
    pub n_costs: u8,
    /// What the deployable's own recipe consumes, `(item index, units)` —
    /// the raw materials, not the one crafted item placement takes.
    ///
    /// Placement charges one `item`; repair cannot, because a fractional
    /// share of one item rounds to the whole thing and a scratched door
    /// would cost a whole door. So a repair is priced against what the
    /// door was *made* of, pro-rata, exactly as a piece is priced against
    /// `PieceDef::costs` — one formula, two stores.
    pub costs: [(u16, u16); MAX_DEPLOY_COSTS],
}

impl DeployDef {
    pub const INERT: Self = Self {
        arch: ARCH_BAG,
        placement: PLACE_GROUND,
        hp: 0,
        item: 0,
        n_costs: 0,
        costs: [(0, 0); MAX_DEPLOY_COSTS],
    };
}

/// The whole deployable ruleset the sim knows, plus the upkeep globals
/// that price decay (baked from `content/balance.toml` §globals and the
/// build table's cost items). Construction input like the gather table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeployContent {
    pub defs: [DeployDef; MAX_DEPLOY_DEFS],
    pub def_count: u16,
    /// Distinct upkeep materials (item indices, ascending) — the union of
    /// the build table's cost items. Hearth stock rows align to these.
    pub mats: [u16; HEARTH_STOCK_ROWS],
    pub mat_count: u8,
    /// `content/balance.toml` globals.upkeep_pct_per_day.
    pub upkeep_pct_per_day: u16,
    /// Unpaid decay per period as a % of max hp, indexed by
    /// `build::MAT_*` (upkeep/decay v1). A zero entry means the content
    /// priced no ladder and `DECAY_PCT_PER_PERIOD` answers instead, which
    /// is what keeps an older `balance.toml` playing the game it played.
    pub decay_pct: [u16; DECAY_MATERIALS],
}

impl DeployContent {
    /// Inert: no deployable exists, every request refuses, nothing
    /// decays (no materials priced). `World::new` starts here.
    pub const EMPTY: Self = Self {
        defs: [DeployDef::INERT; MAX_DEPLOY_DEFS],
        def_count: 0,
        mats: [0; HEARTH_STOCK_ROWS],
        mat_count: 0,
        upkeep_pct_per_day: 0,
        decay_pct: [0; DECAY_MATERIALS],
    };

    /// Synthetic table for the parity/replay/alloc gates, over the gather
    /// probe fixture's items (fixture, not game content). Rows cover the
    /// hearth, a station, a door, and a ground deploy so every placement
    /// class and the claim/upkeep paths ride the gates.
    ///
    /// Repair prices are items 0 and 1 — the build fixture's cost items,
    /// so an inventory stocked for building can already pay for mending.
    /// The door carries two rows on purpose: a multi-material deployable
    /// repair is the case where a half-paid payment would show. The bag
    /// carries none, also on purpose, so `REFUSE_B_UNPRICED` has a live
    /// target inside the gates rather than only in a unit test.
    pub fn probe_fixture() -> Self {
        let mut d = Self::EMPTY;
        d.def_count = 8;
        d.defs[0] = DeployDef {
            arch: ARCH_HEARTH,
            placement: PLACE_FOUNDATION,
            hp: 100,
            item: 2,
            n_costs: 2,
            costs: [(0, 30), (1, 10), (0, 0), (0, 0)],
        };
        d.defs[1] = DeployDef {
            arch: ARCH_WORKBENCH,
            placement: PLACE_ANY,
            hp: 80,
            item: 3,
            n_costs: 1,
            costs: [(0, 20), (0, 0), (0, 0), (0, 0)],
        };
        d.defs[2] = DeployDef {
            arch: ARCH_DOOR,
            placement: PLACE_DOORWAY,
            hp: 60,
            item: 4,
            n_costs: 2,
            costs: [(0, 40), (1, 8), (0, 0), (0, 0)],
        };
        d.defs[3] = DeployDef {
            arch: ARCH_BAG,
            placement: PLACE_GROUND,
            hp: 50,
            item: 5,
            n_costs: 0,
            costs: [(0, 0); MAX_DEPLOY_COSTS],
        };
        // The oven (oven v0): a body deployable on the ground, so the
        // gates can place one, feed it the fixture's fuel (item 0) and
        // watch it burn. `CookContent::probe_fixture` is the other half —
        // item 0 burns to item 5, item 1 cooks into item 6 — and the two
        // fixtures are only meaningful together.
        d.defs[4] = DeployDef {
            arch: ARCH_FIRE,
            placement: PLACE_GROUND,
            hp: 40,
            item: 6,
            n_costs: 1,
            costs: [(0, 25), (0, 0), (0, 0), (0, 0)],
        };
        // The code lock (lock v1). In the probe fixture rather than only
        // in `content/` because the parity, alloc and replay gates all
        // install *this* table, and a wall that cannot see a verb is not
        // a wall — the lock ops have to run inside every one of them.
        //
        // Item **7**, not 6: the oven above already spends 6, and two
        // deployables sharing an item would make `lock_row`'s give-back
        // hand out an oven.
        d.defs[5] = DeployDef {
            arch: ARCH_LOCK,
            placement: PLACE_DOOR,
            hp: 40,
            item: 7,
            n_costs: 1,
            costs: [(1, 12), (0, 0), (0, 0), (0, 0)],
        };
        // The recycler (recycler v0), here for the lock's reason exactly:
        // the parity, alloc and replay gates install *this* table, and a
        // wall that cannot see a verb is not a wall. `CookContent::
        // probe_fixture` is the other half — item 2 recycles into items 6
        // and 7 on ONE timer, which is the multi-row conversion path and
        // the newest arithmetic in `oven.rs`.
        //
        // Item **8**, the ninth: 0–7 are all spoken for above and by the
        // build fixture's cost items, so `gather::GatherContent::
        // probe_fixture` widened to nine to make room.
        d.defs[6] = DeployDef {
            arch: ARCH_RECYCLER,
            placement: PLACE_GROUND,
            hp: 45,
            item: 8,
            n_costs: 1,
            costs: [(1, 15), (0, 0), (0, 0), (0, 0)],
        };
        // The research table (research v0), here for the recycler's and
        // the lock's reason: the parity, alloc and replay gates install
        // *this* table, and a verb no wall can see is not walled.
        // `ResearchContent::probe_fixture` is the other half — item 4
        // unlocks craft recipe 2, which is the blueprint-gated one.
        //
        // Item **10**: 8 is the recycler's and 9 is the box a unit-test
        // fixture appends past the shared set (`boxed_fixture`).
        d.defs[7] = DeployDef {
            arch: ARCH_RESEARCH,
            placement: PLACE_GROUND,
            hp: 45,
            item: 10,
            n_costs: 1,
            costs: [(0, 18), (0, 0), (0, 0), (0, 0)],
        };
        // The build probe fixture costs items 0 and 1.
        d.mats = [0, 1, 0, 0];
        d.mat_count = 2;
        d.upkeep_pct_per_day = 10;
        // A ladder in the probe fixture, so the parity/replay/alloc gates
        // walk the *keyed* path rather than the fallback — a wall that
        // only ever sees the default is not watching the feature. Twig
        // leads at 100: one period and a scaffold is gone.
        d.decay_pct = [100, 34, 20, 13];
        d
    }
}

/// One placed deployable. Grid-addressed like a piece; `owner` gates
/// hearth privilege (and the later respawn-on-bag lane), never the wire.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DeployRec {
    pub cx: u16,
    pub cz: u16,
    pub level: u8,
    pub loc: u8,
    /// Baked deployable row this address holds.
    pub row: u8,
    pub owner: u32,
    /// Current hp (decay drains it; damage lands in M2).
    pub hp: u16,
    /// Last upkeep period processed (`tick / UPKEEP_PERIOD_TICKS`).
    pub uh: u16,
    /// Door state (ARCH_DOOR only; false for everything else). Doors
    /// place closed; the use action toggles. Sim state, hashed, and on
    /// the wire (the deploy record's open bit, wire v6).
    pub open: bool,
    /// Whether a code lock is bolted to this address (ARCH_DOOR only;
    /// false for everything else). A **mirror** of `lock.rs`'s store, not
    /// the truth — the truth is a `LockRec`, and this bit exists so the
    /// wire and the client can draw the door without ever seeing a code
    /// or a remembered list. Sim state, hashed, on the wire (wire v19).
    pub has_lock: bool,
    /// Whether that lock is locked. The other mirror bit, and the one
    /// that decides whether `lock::LockRec::passes` is consulted at all.
    /// `has_lock == false` implies this is false; the pair is kept in
    /// lockstep by every lock verb and re-derived by
    /// `World::rebuild_doors` after a load, exactly as the collision
    /// index's shut bits are.
    pub locked: bool,
    /// The damage band this record was **sent** at
    /// (`build::damage_band`, 0 = untouched). Wire v44.
    ///
    /// ⚠ Wire-only and filled at encode, exactly as `build::PieceRec::dmg`
    /// is — that field carries the full reasoning, including why the store
    /// deliberately does not maintain this and why it is absent from
    /// `state_hash`. Read it on a client, never on a shard.
    pub dmg: u8,
}

/// One of your own bags, as the death screen needs to know it: where it
/// stands and whether its cooldown has lapsed.
///
/// **Not a `DeployRec` subset for a reason.** A deploy record is a world
/// fact and rides a broadcast; this is an **own-fact** — `owner` is
/// deliberately not on the wire (`DeployRec`'s own doc), so a client cannot
/// derive which beds are its own from the mirror it already holds, and the
/// alternative to this type is putting an owner id on every deployable
/// anyone can see. `ready` is likewise never broadcast: which of a
/// defender's bags is spent is exactly what a raider would like to know.
///
/// No `owner` field: the message carries only the recipient's own bags, so
/// an owner column would be one value repeated and one more thing that
/// could disagree with the audience it was sent to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BagAnchor {
    pub cx: u16,
    pub cz: u16,
    pub level: u8,
    /// Whether `claim_bag` would take this one **at the tick it was read**.
    /// A cooldown lapses on a clock nothing announces, so this is a
    /// snapshot and not a subscription — see `own_bags`.
    pub ready: bool,
}

/// The hearth's crew list — who may build inside its claim. Named so
/// `worldsave` and `state_hash` can spell it without restating the cap.
pub type CrewList = Roster<HEARTH_CREW_CAP>;

/// One hearth's claim + stock + **crew**, in the dense hearth list.
/// Identity is the grid address of its deploy record; stock rows align to
/// `DeployContent::mats`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HearthRec {
    pub cx: u16,
    pub cz: u16,
    pub level: u8,
    /// Who placed it. Kept for the reason `LockRec::owner` is kept —
    /// somebody has to be the one it came from — and, like that one, it
    /// is **not** the access check. `crew` is.
    pub owner: u32,
    pub stock: [u32; HEARTH_STOCK_ROWS],
    /// Who may build, upgrade, repair and deploy inside the claim
    /// (hearth crew v1, `reference/BUILDING.md` §9.1). Placing the hearth
    /// joins its crew, which is the reference's own rule — the act of
    /// putting the cupboard down is the act of joining its list (§1 fact
    /// 4) — so this is never empty on a live record.
    pub crew: CrewList,
}

/// One deployed box's contents, in the dense box list. Identity is the
/// grid address of its deploy record — `(cx, cz, level)`, the same triple
/// a hearth uses — because a box is furniture and has no id to hand out.
/// `loc` is deliberately absent: two boxes never share a cell and a level
/// (placement's occupancy check forbids it), so the triple is already
/// unique, and leaving `loc` out is what lets the client name a box from
/// the deploy record it already drew.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BoxRec {
    pub cx: u16,
    pub cz: u16,
    pub level: u8,
    /// Who placed it. Sim-side only, and — unlike a door — **not** an
    /// access check: see `inventory::CONT_BOX`. Kept because the spill a
    /// broken box leaves has to belong to someone, the way a corpse's bag
    /// does.
    pub owner: u32,
    pub items: [ItemStack; BOX_SLOTS],
}

impl Default for BoxRec {
    fn default() -> Self {
        Self {
            cx: 0,
            cz: 0,
            level: 0,
            owner: 0,
            items: [ItemStack::default(); BOX_SLOTS],
        }
    }
}

impl BoxRec {
    pub fn is_empty(&self) -> bool {
        self.items.iter().all(|s| s.count == 0)
    }
}

/// Pack a box's grid address into the one container handle the move
/// command carries. `cx`/`cz` are bounded by `MAX_BUILD_COORD` (1024, so
/// ten bits each) and `level` by `MAX_BUILD_LEVELS` (8, three bits), which
/// is 23 bits inside a `u32` with room to spare.
///
/// Deliberately **not** `gather::cell_key`: that one packs `cx << 16 | cz`
/// and spends all 32 bits on the pair, leaving nowhere for the level. A
/// box on a second storey is a box, so the level is part of the address.
pub fn box_key(cx: u16, cz: u16, level: u8) -> u32 {
    ((cx as u32) << 16) | ((cz as u32) << 4) | (level as u32 & 0xF)
}

/// The contents of every standing box, plus the spill buffer a removal
/// hands to `world.rs`. Boxed inside `Deploys` — `MAX_BOXES * BOX_SLOTS`
/// stacks is 12 kB of fixed capacity and `World` is built on the stack
/// (`ShardCore::new`, every wire test) where it is already tight, so this
/// takes the same one-allocation-at-construction posture `Backpacks`
/// takes for exactly the same reason. Nothing here allocates in the tick
/// (wall 2).
pub struct BoxStore {
    /// Boxed via [`crate::boxed_array`] since `ItemStack` grew `cond`:
    /// `Box::new(BoxStore::new())` materialises the whole struct in a
    /// frame first, and 256 records of 12 six-byte stacks crosses the
    /// wasm shadow-stack line `CLAUDE.md`'s trap entry measures — the
    /// symptom is `test_parity_wasm` dying as an out-of-bounds read with
    /// every native test green.
    entries: Box<[BoxRec; MAX_BOXES]>,
    len: usize,
    /// Oven state, index-aligned to `entries` (`oven.rs`). Every
    /// container carries a row and a storage box's says `ARCH_BOX`, which
    /// is how `is_converter` answers without a second lookup.
    ///
    /// A parallel array for `bag_ready`'s reason, one layer down: the box
    /// record is what the container-sync message is built from, so a burn
    /// timer and twelve cook counters may not ride on it — the client
    /// draws a fire, it does not adjudicate one. Kept aligned by the two
    /// places that can move a box: the insert in `place_deploy` and the
    /// swap-remove in `remove_at`. Boxed with `entries`, for its reason.
    ovens: Box<[crate::oven::OvenState; MAX_BOXES]>,
    /// Contents of boxes removed this tick, awaiting a ground bag.
    /// `drop_deploy` is reached from decay and from a raid and holds
    /// neither the bag store nor the clock, so it parks the record here
    /// and `World::step` stands the bag up before the tick ends.
    spill: [BoxRec; MAX_BOX_SPILL_PER_TICK],
    spill_len: usize,
}

impl BoxStore {
    fn new() -> Self {
        Self {
            entries: crate::boxed_array(BoxRec::default()),
            len: 0,
            ovens: crate::boxed_array(crate::oven::OvenState::default()),
            spill: [BoxRec::default(); MAX_BOX_SPILL_PER_TICK],
            spill_len: 0,
        }
    }
}

/// The placed-deployable store: dense, insertion-ordered, plus the dense
/// hearth list the claim checks and the sweep scan, and the dense box list
/// the move verb resolves against. Removal swap-removes (decay order is
/// the sweep cursor's, deterministic); the wire layer restarts in-progress
/// sync walks on any removal.
pub struct Deploys {
    /// Boxed, like every large array here, so `Deploys::new` never puts
    /// one in a stack frame — [`crate::boxed_array`] has the measurement
    /// and the wasm failure it prevents.
    entries: Box<[DeployRec; MAX_DEPLOYS]>,
    len: usize,
    /// Bag respawn cooldowns, index-aligned to `entries`: the first tick
    /// `entries[i]` may be woken on again. Meaningless for every archetype
    /// that is not `ARCH_BAG`, and zero for a fresh one — `0` is "ready from
    /// the first tick", not a sentinel.
    ///
    /// A parallel array rather than a field on `DeployRec`, and that is a
    /// wire decision rather than a layout preference: `DeployRec` is the
    /// struct the deploy-sync packet mirrors, so eight bytes of sim-only
    /// timer on it would ride 24-deep in `EventMsg::DeploySync` and grow the
    /// client's event enum by 192 bytes it can never read. The client draws
    /// a bag; it does not adjudicate one. Kept aligned by `insert` and by
    /// `remove_at`, whose swap-remove moves both halves together.
    bag_ready: Box<[u64; MAX_DEPLOYS]>,
    /// When each entry was placed, index-aligned to `entries` — the
    /// pickup window's clock. A parallel array for `bag_ready`'s reason,
    /// stated one field up: `DeployRec` is what the deploy-sync packet
    /// mirrors.
    placed: Box<[u64; MAX_DEPLOYS]>,
    hearths: Box<[HearthRec; MAX_HEARTHS]>,
    hearth_count: usize,
    /// The box contents, on the heap. Same wire decision as `bag_ready`
    /// above and the same one as the hearth list: `DeployRec` is what the
    /// deploy-sync packet mirrors, so a box's twelve stacks may not ride
    /// on it.
    boxes: Box<BoxStore>,
    /// The code locks bolted onto doors (lock v1, `lock.rs`). Third store
    /// with the same justification as the two above, and the strongest
    /// case of the three: a lock carries two codes, two remembered lists
    /// and two tick counters, none of which a client may see.
    ///
    /// Not `Box<Locks>` — `Locks` boxes its own array, for the shadow-stack
    /// reason its doc comment records.
    locks: Locks,
    /// The privilege volumes, cached per hearth — the shape the upkeep
    /// sweep's coverage questions read (`claim::ClaimCache`, whose doc
    /// carries the determinism and bounds arguments whole). Derived state
    /// like `Pieces::cols`: never hashed, never saved, rebuilt by
    /// [`Deploys::refresh_claims`] when the stamps below say it is stale.
    claim: Box<crate::claim::ClaimCache>,
    /// Bumped by every hearth add, removal and restore — `Pieces::gen`'s
    /// twin for the hearth list, and like it derived-cache plumbing
    /// rather than state: never hashed, never saved, deterministic
    /// anyway (every bump site is stream-ordered).
    hearth_gen: u64,
}

impl Deploys {
    pub fn new() -> Self {
        Self {
            entries: crate::boxed_array(DeployRec::default()),
            len: 0,
            bag_ready: crate::boxed_array(0),
            placed: crate::boxed_array(0),
            hearths: crate::boxed_array(HearthRec::default()),
            hearth_count: 0,
            boxes: Box::new(BoxStore::new()),
            locks: Locks::new(),
            claim: crate::claim::ClaimCache::new(),
            hearth_gen: 0,
        }
    }

    /// Rebuild the claim cache if a piece or a hearth changed since it
    /// was last built. Called from exactly one place — the top of
    /// [`upkeep_sweep`], the fixed point in the tick the determinism
    /// argument in `claim.rs` names — and from the gates.
    pub(crate) fn refresh_claims(&mut self, pieces: &Pieces) {
        if self
            .claim
            .fresh_for(pieces.footprint_gen(), self.hearth_gen)
        {
            return;
        }
        let hg = self.hearth_gen;
        self.claim.rebuild(
            pieces,
            &self.hearths[..self.hearth_count],
            pieces.footprint_gen(),
            hg,
        );
    }

    /// Whether hearth `hi`'s **cached** claim volume covers the planar
    /// point — the upkeep sweep's coverage question, and since the cache
    /// landed the only coverage question the sweep asks: the base's own
    /// shape, not a circle. Callers hold the cache fresh via
    /// [`Deploys::refresh_claims`].
    pub(crate) fn hearth_covers(&self, hi: usize, x: f32, z: f32) -> bool {
        self.claim.covers(hi, x, z)
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn entries(&self) -> &[DeployRec] {
        &self.entries[..self.len]
    }

    /// The live half of `bag_ready`, index-aligned to `entries()`. Read by
    /// `state_hash` (it is sim state) and by the gates that assert a bag was
    /// actually spent.
    pub fn bag_ready(&self) -> &[u64] {
        &self.bag_ready[..self.len]
    }

    /// The live half of `placed`, index-aligned to `entries()`. Sim state
    /// like `bag_ready`, and read by `state_hash` and `worldsave` for the
    /// same reason.
    pub fn placed(&self) -> &[u64] {
        &self.placed[..self.len]
    }

    /// The tick entry `i` was placed on. A stale index reads 0 —
    /// "placed at tick 0", long out of its window, which is the safe
    /// direction because it refuses rather than refunds.
    pub fn placed_at(&self, i: usize) -> u64 {
        if i >= self.len {
            return 0;
        }
        self.placed[i]
    }

    pub fn hearths(&self) -> &[HearthRec] {
        &self.hearths[..self.hearth_count]
    }

    /// Stand a hearth at an address with no placement rules applied.
    /// **Fixtures only**, for `Pieces::insert_for_test`'s reason.
    #[cfg(test)]
    pub(crate) fn push_hearth_for_test(&mut self, cx: u16, cz: u16, level: u8, owner: u32) {
        self.hearths[self.hearth_count] = HearthRec {
            cx,
            cz,
            level,
            owner,
            stock: [0; HEARTH_STOCK_ROWS],
            crew: CrewList::of(owner),
        };
        self.hearth_count += 1;
        self.hearth_gen += 1;
    }

    /// The hearth records, writable. **Tests and fixtures only** — every
    /// live path reaches a crew through `crew_op`, which is where the
    /// rights checks are; a second writer would be a second place for
    /// "may this hand change this list" to be decided.
    #[cfg(test)]
    pub(crate) fn hearths_mut(&mut self) -> &mut [HearthRec] {
        &mut self.hearths[..self.hearth_count]
    }

    /// The live box records. Read by `state_hash` (contents are sim
    /// state) and by the gates.
    pub fn boxes(&self) -> &[BoxRec] {
        &self.boxes.entries[..self.boxes.len]
    }

    /// The live code locks. Read by `state_hash` (a code and a remembered
    /// list are sim state as much as a box's contents are), by
    /// `worldsave`, and by the gates.
    pub fn locks(&self) -> &[LockRec] {
        self.locks.entries()
    }

    /// Write the two mirror bits of `entries[i]` from the lock store's
    /// verdict. `World::rebuild_doors` is the only caller and the doc
    /// there is the argument; it exists as a method because `entries` is
    /// private and a load must not be able to set them independently of
    /// each other (`has_lock == false` implies `locked == false`).
    pub(crate) fn set_lock_mirror(&mut self, i: usize, has_lock: bool, locked: bool) {
        self.entries[i].has_lock = has_lock;
        self.entries[i].locked = has_lock && locked;
    }

    /// Whether the lock at this address (if any) lets `id` work the leaf.
    /// **The one access question**, so `use_door`, the client's own mirror
    /// and any later container check all read the same answer.
    pub fn lock_passes(&self, cx: u16, cz: u16, level: u8, loc: u8, id: u32) -> bool {
        self.locks.passes(cx, cz, level, loc, id)
    }

    /// Replace every store from a decoded world save. Boot-only
    /// (`worldsave.rs`).
    ///
    /// Seven arrays because the store is seven arrays, and the file writes
    /// each parallel one inline on the record it belongs to — so the one
    /// invariant worth stating is the one the caller has to hold up:
    /// `ready` is index-aligned to `recs` and `ovens` to `boxes`, exactly
    /// as the fields they land in are index-aligned to their entries. A
    /// shorter `ready` leaves the tail at zero, which reads as "ready from
    /// the first tick" and not as a sentinel, so the failure would be
    /// silently generous bags rather than a panic. The decoder writes them
    /// as one record and cannot produce that; the `debug_assert`s are here
    /// so a second caller cannot either.
    ///
    /// The argument count is over clippy's line and stays that way rather
    /// than being bundled into a struct: every one of these is a decoded
    /// slice the caller already has laid out separately, and a wrapper
    /// would be a type that exists for one call site and hides which
    /// slices have to be aligned to which.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn restore(
        &mut self,
        recs: &[DeployRec],
        ready: &[u64],
        placed: &[u64],
        hearths: &[HearthRec],
        boxes: &[BoxRec],
        ovens: &[crate::oven::OvenState],
        locks: &[LockRec],
    ) {
        debug_assert_eq!(recs.len(), ready.len(), "bag_ready must be index-aligned");
        self.len = recs.len().min(MAX_DEPLOYS);
        self.entries[..self.len].copy_from_slice(&recs[..self.len]);
        self.bag_ready[..self.len].copy_from_slice(&ready[..self.len]);
        self.placed[..self.len].copy_from_slice(&placed[..self.len]);
        self.hearth_count = hearths.len().min(MAX_HEARTHS);
        self.hearths[..self.hearth_count].copy_from_slice(&hearths[..self.hearth_count]);
        // The loaded list replaced the live one wholesale; the bump makes
        // the first sweep after a load rebuild the claim cache.
        self.hearth_gen += 1;
        debug_assert_eq!(boxes.len(), ovens.len(), "oven state must be index-aligned");
        self.boxes.len = boxes.len().min(MAX_BOXES);
        self.boxes.entries[..self.boxes.len].copy_from_slice(&boxes[..self.boxes.len]);
        self.boxes.ovens[..self.boxes.len].copy_from_slice(&ovens[..self.boxes.len]);
        self.locks.len = locks.len().min(MAX_LOCKS);
        self.locks.entries[..self.locks.len].copy_from_slice(&locks[..self.locks.len]);
        // The spill list is within-tick scratch (`BoxStore`), never state:
        // a save taken between ticks cannot hold one, and a load must not
        // leave a stale one for `World::step` to stand a bag up from.
        self.boxes.spill_len = 0;
    }

    /// The live oven states, index-aligned to `boxes()`. Read by
    /// `state_hash` (a burn timer is sim state), by `worldsave.rs`, and
    /// by the gates.
    pub fn oven_states(&self) -> &[crate::oven::OvenState] {
        &self.boxes.ovens[..self.boxes.len]
    }

    /// The two halves an oven step writes, as live slices. One call
    /// rather than two accessors because a step reads a box's slots and
    /// writes its state in the same breath, and handing them out
    /// separately would be handing out two `&mut` to one store.
    pub(crate) fn oven_parts_mut(&mut self) -> (&mut [BoxRec], &mut [crate::oven::OvenState]) {
        let n = self.boxes.len;
        (&mut self.boxes.entries[..n], &mut self.boxes.ovens[..n])
    }

    /// Resolve a packed address to a container index whose state says
    /// **converter** — an oven or a recycler, the two things a use press
    /// switches on. `box_index`'s filter, and the whole of what `world.rs`
    /// needs to route a use press between the door verb and the oven
    /// verb: the two never share an address (a door lives on a doorway's
    /// edge, an oven on the plane), so the routing is a lookup and not a
    /// guess about what the player aimed at.
    ///
    /// A plain storage box is deliberately excluded and that is the whole
    /// filter: pressing E at a box opens the container panel, and a box
    /// that answered here would have a switch nothing in the sim reads.
    pub fn oven_index(&self, key: u32) -> Option<usize> {
        self.box_index(key)
            .filter(|&i| self.boxes.ovens[i].is_converter())
    }

    /// Resolve a packed box address to an index into `boxes()`, or `None`
    /// when no box stands there. The index is transient — swap-remove
    /// invalidates it — which is exactly why the handle on the wire is the
    /// address and never this.
    ///
    /// Handle 0 is not a box, here or anywhere: it is what every layer
    /// already says when it means *no container open* (`bridge.rs`'s
    /// "nothing is open", `core.rs`'s server-side close, `CONT_SELF`'s
    /// zeroed handle field). `box_key(0, 0, 0)` packs to 0 and would
    /// otherwise be a real address, so the two readings collide on one
    /// cell — and the client, unable to tell them apart, has to refuse a
    /// ground handle of 0 outright. The collision is closed on both sides
    /// the way `Backpacks` already closes it: **the handle is never
    /// minted** (`place_deploy` refuses a box at that address with
    /// `REFUSE_D_SPOT`, as `next_id: 1` refuses it for bags) and **the
    /// decode guards it** (here, as `index_of_id` does). Either half alone
    /// leaves one side unable to trust the other.
    pub fn box_index(&self, key: u32) -> Option<usize> {
        if key == 0 {
            return None;
        }
        self.boxes.entries[..self.boxes.len]
            .iter()
            .position(|b| box_key(b.cx, b.cz, b.level) == key)
    }

    /// One slot of box `i`. Total: an index or slot out of range reads
    /// empty, the same posture `Backpacks::slot` takes, so a stale address
    /// can never panic the sim thread.
    pub fn box_slot(&self, i: usize, s: usize) -> ItemStack {
        if i >= self.boxes.len || s >= BOX_SLOTS {
            return ItemStack::default();
        }
        self.boxes.entries[i].items[s]
    }

    /// Write one slot of box `i`. Total in the same way: out of range
    /// writes nothing.
    pub fn set_box_slot(&mut self, i: usize, s: usize, stack: ItemStack) {
        if i >= self.boxes.len || s >= BOX_SLOTS {
            return;
        }
        self.boxes.entries[i].items[s] = stack;
    }

    /// Planar distance from a box to a player, against the same
    /// `BUILD_REACH_M` a door uses — and planar for the same reason: a
    /// door on the storey above is reachable today too, and inventing a
    /// vertical rule for boxes alone would make two verbs disagree about
    /// what "in reach" means. Level is part of the *address*, not of the
    /// reach test.
    pub fn box_in_reach(&self, i: usize, p: &Player) -> bool {
        if i >= self.boxes.len {
            return false;
        }
        let b = self.boxes.entries[i];
        let (bx, bz) = cell_center(b.cx, b.cz);
        let (px, pz) = player_xz(p);
        let (dx, dz) = (bx - px, bz - pz);
        dx * dx + dz * dz <= crate::build::BUILD_REACH_M * crate::build::BUILD_REACH_M
    }

    /// How many spills this tick's removals left. Read once per tick by
    /// `World::step`, which owns the bag store and the clock.
    pub fn box_spill_len(&self) -> usize {
        self.boxes.spill_len
    }

    /// One parked spill. Read by value — the caller stands a bag up from
    /// it and never holds a reference into the store.
    pub fn box_spill_at(&self, i: usize) -> BoxRec {
        if i >= self.boxes.spill_len {
            return BoxRec::default();
        }
        self.boxes.spill[i]
    }

    /// Empty the spill buffer, after `World::step` has stood every parked
    /// record up. Separate from the reads so a spill is drained exactly
    /// once and never half-drained by an early return.
    pub fn clear_box_spill(&mut self) {
        self.boxes.spill_len = 0;
    }

    pub fn find(&self, cx: u16, cz: u16, level: u8, loc: u8) -> Option<&DeployRec> {
        self.entries[..self.len]
            .iter()
            .find(|d| d.cx == cx && d.cz == cz && d.level == level && d.loc == loc)
    }

    /// The store index of the deployable at an address — `Pieces`'
    /// `find_index`, for the same reason: `find` hands out a reference the
    /// borrow checker will not let a mutation follow. The address is
    /// shared with the piece store by design (a door and its doorway have
    /// one address), so the caller has already decided which store it
    /// means; this one never guesses.
    pub(crate) fn find_index(&self, cx: u16, cz: u16, level: u8, loc: u8) -> Option<usize> {
        self.entries[..self.len]
            .iter()
            .position(|d| d.cx == cx && d.cz == cz && d.level == level && d.loc == loc)
    }

    /// Write entry `i`'s hp alone — the repair verb's write (`build.rs`).
    /// `Pieces::set_hp`'s twin, and the upkeep clock is untouched for its
    /// reason: materials are not rent. Until this existed every hp write
    /// on a deployable was a direct field poke inside this module, which
    /// is why the verb could not reach a door from outside it.
    pub(crate) fn set_hp(&mut self, i: usize, hp: u16) {
        self.entries[i].hp = hp;
    }

    fn insert(&mut self, rec: DeployRec, tick: u64) -> bool {
        if self.len == MAX_DEPLOYS {
            return false;
        }
        self.entries[self.len] = rec;
        // A bag is born ready: place one and the next death is answered.
        self.bag_ready[self.len] = 0;
        self.placed[self.len] = tick;
        self.len += 1;
        true
    }

    /// Swap-remove entry `i`; drops the hearth record too when the entry
    /// was a hearth. The caller announces the removal event.
    fn remove_at(&mut self, i: usize, dc: &DeployContent) {
        let rec = self.entries[i];
        self.len -= 1;
        self.entries[i] = self.entries[self.len];
        // Every half moves together or a parallel array stops describing
        // the record it is indexed against.
        self.bag_ready[i] = self.bag_ready[self.len];
        self.placed[i] = self.placed[self.len];
        if dc.defs[rec.row as usize].arch == ARCH_HEARTH {
            if let Some(h) = self.hearths[..self.hearth_count]
                .iter()
                .position(|h| h.cx == rec.cx && h.cz == rec.cz && h.level == rec.level)
            {
                self.hearth_count -= 1;
                self.hearths[h] = self.hearths[self.hearth_count];
                // The cache row moves with the record it describes —
                // mid-tick, before any rebuild, a query for hearth `h`
                // must not read the removed hearth's volume.
                self.claim.hearth_swap_remove(h, self.hearth_count);
                self.hearth_gen += 1;
            }
        }
        if holds_items(dc.defs[rec.row as usize].arch) {
            if let Some(b) = self.box_index(box_key(rec.cx, rec.cz, rec.level)) {
                let bx = self.boxes.entries[b];
                self.boxes.len -= 1;
                let last = self.boxes.len;
                self.boxes.entries[b] = self.boxes.entries[last];
                self.boxes.entries[last] = BoxRec::default();
                // Both halves move together, or the oven states stop
                // describing the containers they are indexed against —
                // `bag_ready`'s invariant one store down.
                self.boxes.ovens[b] = self.boxes.ovens[last];
                self.boxes.ovens[last] = crate::oven::OvenState::default();
                // What was inside outlives the box. Parked rather than
                // dropped where it stands, because this path is reached
                // from decay and from a raid and holds neither the bag
                // store nor the clock; `World::step` drains it this same
                // tick. An empty box parks nothing — `stand_up` would
                // refuse it anyway, and a full buffer is then never spent
                // on boxes that had nothing in them.
                if !bx.is_empty() && self.boxes.spill_len < MAX_BOX_SPILL_PER_TICK {
                    self.boxes.spill[self.boxes.spill_len] = bx;
                    self.boxes.spill_len += 1;
                }
            }
        }
    }

    /// First hearth (list order) with any stock whose **cached claim
    /// volume** covers the planar point — the deployable half of the
    /// sweep's coverage test, asked of the base's own shape rather than
    /// `HEARTH_RADIUS_M` since the cache landed. An empty hearth covers
    /// nothing (itself included), unchanged from the circle days; stock
    /// is read live because the piece half of the same sweep spends it
    /// between visits, and geometry is the cache's because nothing inside
    /// a sweep visit moves structure.
    fn covering_hearth(&self, x: f32, z: f32) -> Option<usize> {
        (0..self.hearth_count).find(|&hi| {
            self.hearths[hi].stock.iter().any(|&s| s != 0) && self.hearth_covers(hi, x, z)
        })
    }

    /// Whether a *foreign* hearth's **legacy circle** claims the planar
    /// point. No verb asks this any more — placement, upgrade, repair and
    /// the sweep all answer to the base's own volume (`claim.rs`) — and
    /// it survives as the crew tests' probe: pure who-is-on-the-list
    /// semantics with the simplest geometry there is.
    pub fn foreign_claim(&self, x: f32, z: f32, placer: u32) -> bool {
        let r2 = HEARTH_RADIUS_M * HEARTH_RADIUS_M;
        self.hearths[..self.hearth_count].iter().any(|h| {
            // The crew, not the owner (hearth crew v1). This one line is
            // the whole of "a base can be shared": before it, `h.owner ==
            // placer` made every claim answer to exactly one id, which is
            // lock v0's bug living in the building system
            // (`reference/BUILDING.md` §9.1).
            if h.crew.contains(placer) {
                return false;
            }
            let (hx, hz) = cell_center(h.cx, h.cz);
            let (dx, dz) = (hx - x, hz - z);
            dx * dx + dz * dz <= r2
        })
    }

    /// Whether any placed deployable of `arch` sits within `radius_m`
    /// (planar) of the point — the craft station check.
    pub fn arch_near(&self, dc: &DeployContent, arch: u8, x: f32, z: f32, radius_m: f32) -> bool {
        let r2 = radius_m * radius_m;
        self.entries[..self.len].iter().any(|d| {
            if dc.defs[d.row as usize].arch != arch {
                return false;
            }
            let (ax, az) = cell_center(d.cx, d.cz);
            let (dx, dz) = (ax - x, az - z);
            dx * dx + dz * dz <= r2
        })
    }

    /// Whether any placed workbench of tier ≥ `tier` sits within
    /// `radius_m` (planar) of the point — the tiered station check
    /// (bench ladder v0). One scan rather than one `arch_near` per rung,
    /// and the ≥ is the reference's own rule: a level-3 bench crafts a
    /// level-1 recipe, so upgrading a bench never costs a verb.
    pub fn bench_near(&self, dc: &DeployContent, tier: u8, x: f32, z: f32, radius_m: f32) -> bool {
        let r2 = radius_m * radius_m;
        self.entries[..self.len].iter().any(|d| {
            if bench_tier(dc.defs[d.row as usize].arch) < tier {
                return false;
            }
            let (ax, az) = cell_center(d.cx, d.cz);
            let (dx, dz) = (ax - x, az - z);
            dx * dx + dz * dz <= r2
        })
    }

    fn own_bag_count(&self, dc: &DeployContent, owner: u32) -> usize {
        self.entries[..self.len]
            .iter()
            .filter(|d| d.owner == owner && dc.defs[d.row as usize].arch == ARCH_BAG)
            .count()
    }

    /// Every bag this owner has placed, as anchors, newest last — the
    /// **same filter `claim_bag` scans** and therefore the same set the
    /// death screen may offer.
    ///
    /// `out` is filled from the front and the live count returned;
    /// `BAG_CAP` is the ceiling placement already enforces
    /// (`own_bag_count` above), so a full array is a full store and never
    /// a truncation — the `break` is wall 4's cap check, not a policy.
    ///
    /// ⚠ **This is a READ, and it never spends a bag.** `claim_bag` is the
    /// verb; this is the picture of it. Keeping them apart is what lets a
    /// client be told about a bag without the world deciding it was used —
    /// the same split `the_beach_button_refuses_a_ready_bag` gates one
    /// layer up.
    ///
    /// Bounded and allocation-free: one pass of the store, one compare
    /// against `tick` per bag, and no iteration order that is not the
    /// store's own (wall 1).
    pub fn own_bags(
        &self,
        dc: &DeployContent,
        owner: u32,
        tick: u64,
        out: &mut [BagAnchor; BAG_CAP],
    ) -> usize {
        let mut n = 0;
        for (i, d) in self.entries[..self.len].iter().enumerate() {
            if d.owner != owner || dc.defs[d.row as usize].arch != ARCH_BAG {
                continue;
            }
            if n == BAG_CAP {
                break;
            }
            out[n] = BagAnchor {
                cx: d.cx,
                cz: d.cz,
                level: d.level,
                ready: self.bag_ready[i] <= tick,
            };
            n += 1;
        }
        n
    }

    /// The bag a dying player wakes on, and the cooldown that spends it.
    ///
    /// Scans `owner`'s own ready bags and returns the planar cell center of
    /// the one **nearest the point given** — the body's position as it
    /// fell, so a death defending a compound wakes on the bag inside that
    /// compound rather than on one across the island. Ties go to the
    /// earlier entry, which is the store's insertion order and therefore
    /// the same on every replay of the same log.
    ///
    /// The chosen bag is *spent*: its `bag_ready` entry moves to
    /// `tick + BAG_COOLDOWN_TICKS`, and until then every scan skips it. So
    /// a player killed twice inside the cooldown wakes on a second bag if
    /// they placed one and back on the spawn ring if they did not — which
    /// is the whole reason the cooldown exists, and the reason `BAG_CAP`
    /// bounds how many answers a defender can stack.
    ///
    /// `None` means exactly one thing: no bag of this owner's is ready.
    /// The caller falls back to the ring on it.
    ///
    /// Bounded and allocation-free — one pass of the store, squared planar
    /// distance (monotone in distance, so no `sqrt` and no trig), and no
    /// iteration order that is not the store's own (wall 1).
    pub fn claim_bag(
        &mut self,
        dc: &DeployContent,
        owner: u32,
        x: f32,
        z: f32,
        tick: u64,
    ) -> Option<(f32, f32)> {
        let mut best: Option<(usize, f32)> = None;
        for (i, d) in self.entries[..self.len].iter().enumerate() {
            if d.owner != owner
                || dc.defs[d.row as usize].arch != ARCH_BAG
                || self.bag_ready[i] > tick
            {
                continue;
            }
            let (bx, bz) = cell_center(d.cx, d.cz);
            let (dx, dz) = (bx - x, bz - z);
            let d2 = dx * dx + dz * dz;
            if best.is_none_or(|(_, b)| d2 < b) {
                best = Some((i, d2));
            }
        }
        let (i, _) = best?;
        self.bag_ready[i] = tick.saturating_add(BAG_COOLDOWN_TICKS);
        Some(cell_center(self.entries[i].cx, self.entries[i].cz))
    }
}

impl Default for Deploys {
    fn default() -> Self {
        Self::new()
    }
}

/// Where a broken box's contents fall: the centre of its own cell, on its
/// own storey. The height is `build::column_floor_y` — the one floor
/// formula — plus the storey, so the bag lands ON the floor the box stood
/// on. This function restated the formula until 2026-08-15 and its copy
/// had already drifted: it sampled raw terrain with no lift and no
/// lattice, so the bag sat 0.3 m inside the slab it claimed to land on —
/// the exact hand-kept-mirror failure `column_floor_y` exists to close.
pub fn box_drop_pos(
    seed: u64,
    haven: &terrain::Haven,
    cx: u16,
    cz: u16,
    level: u8,
) -> (f32, f32, f32) {
    let (x, z) = cell_center(cx, cz);
    (
        x,
        crate::build::column_floor_y(seed, haven, cx, cz) + level as f32 * LEVEL_H_M,
        z,
    )
}

/// The point `place_deploy` measures reach to, for every `loc`. `build.rs`
/// measures `repair` to `build::anchor` instead, which is this point for a
/// plane and half a cell off it for an edge; named so the test that
/// pins that relation can name both functions rather than re-deriving one of
/// them and gating its own arithmetic.
///
/// `pub` because the client's deploy ghost is the second caller
/// (`client/ui/place.rs::deploy_verdict`): its reach guess must measure to
/// the same point this verb refuses on, not to a copy of it.
pub fn cell_center(cx: u16, cz: u16) -> (f32, f32) {
    (
        cx as f32 * crate::build::BUILD_CELL_M + crate::build::BUILD_CELL_M * 0.5,
        cz as f32 * crate::build::BUILD_CELL_M + crate::build::BUILD_CELL_M * 0.5,
    )
}

fn player_xz(p: &Player) -> (f32, f32) {
    (
        p.body.qx as f32 * crate::movement::POS_XZ_Q,
        p.body.qz as f32 * crate::movement::POS_XZ_Q,
    )
}

/// Whether `loc` is the kind of slot the placement class occupies.
///
/// `pub` because the client's deploy ghost is the second caller
/// (`client/ui/place.rs::deploy_verdict`): a preview that guessed this rule
/// instead of asking it would be the drift the quantize-both-sides law
/// exists to close.
pub fn loc_fits_placement(placement: u8, loc: u8) -> bool {
    match placement {
        PLACE_DOORWAY => loc == LOC_EDGE_XLO || loc == LOC_EDGE_ZLO,
        // A lock goes where its target lives: a door on a doorway's edge,
        // a box on the plane (locks on boxes, `DOORS.md` §9.8). The
        // support arm below still requires a lockable deployable at the
        // exact address, so this widening admits no empty cell.
        PLACE_DOOR => loc == LOC_EDGE_XLO || loc == LOC_EDGE_ZLO || loc == LOC_PLANE,
        PLACE_GROUND | PLACE_FOUNDATION | PLACE_ANY => loc == LOC_PLANE,
        _ => false,
    }
}

/// Ground-class terrain rule: same buildable shape as a foundation
/// (build.rs consts), and the cell body must be piece-free.
fn ground_ok(seed: u64, haven: &terrain::Haven, pieces: &Pieces, cx: u16, cz: u16) -> bool {
    if pieces.find(cx, cz, 0, LOC_PLANE).is_some() {
        return false;
    }
    let (x, z) = cell_center(cx, cz);
    terrain::ground(seed, haven, x, z) >= crate::build::FOUNDATION_MIN_H_M
        && terrain::ground_slope(seed, haven, x, z) < crate::build::FOUNDATION_MAX_SLOPE
}

/// Apply one deploy-place request (`Command::PlaceDeploy`). Refusals are
/// events, not errors. The deployable's own item is the cost, consumed
/// whole; EV_DEPLOY_PLACED announces the record for the wire.
#[allow(clippy::too_many_arguments)]
pub fn place_deploy(
    seed: u64,
    haven: &terrain::Haven,
    dc: &DeployContent,
    bc: &BuildContent,
    pieces: &mut Pieces,
    deploys: &mut Deploys,
    p: &mut Player,
    tick: u64,
    row: u16,
    cx: u16,
    cz: u16,
    level: u8,
    loc: u8,
    events: &mut EventQueue,
) {
    if row >= dc.def_count {
        events.push(EV_DEPLOY_REFUSED, p.id, REFUSE_D_KIND, 0);
        return;
    }
    let def = &dc.defs[row as usize];
    if def.hp == 0 {
        events.push(EV_DEPLOY_REFUSED, p.id, REFUSE_D_KIND, 0);
        return;
    }
    if (cx as usize) >= MAX_BUILD_COORD
        || (cz as usize) >= MAX_BUILD_COORD
        || (level as usize) >= MAX_BUILD_LEVELS
        || !loc_fits_placement(def.placement, loc)
        // Every class but one wants the address **empty**; the lock wants
        // it occupied, and by a door (its `supported` arm below says so).
        || (def.placement != PLACE_DOOR && deploys.find(cx, cz, level, loc).is_some())
        // The one address a box may not have: `box_key(0, 0, 0)` is 0, and
        // 0 is the reserved "no container" handle (`box_index`). Refusing
        // it here is the minting half of that pair — a box placed at this
        // one cell could be opened by nobody, and a spot that refuses is
        // strictly better than an item store that swallows. It costs one
        // build cell at the world's origin corner and nothing else; every
        // other level of that cell, and every other cell, is unaffected.
        || (def.arch == ARCH_BOX && box_key(cx, cz, level) == 0)
    {
        events.push(EV_DEPLOY_REFUSED, p.id, REFUSE_D_SPOT, 0);
        return;
    }
    let (ax, az) = cell_center(cx, cz);
    let (px, pz) = player_xz(p);
    let (dx, dz) = (ax - px, az - pz);
    if dx * dx + dz * dz > crate::build::BUILD_REACH_M * crate::build::BUILD_REACH_M {
        events.push(EV_DEPLOY_REFUSED, p.id, REFUSE_D_REACH, 0);
        return;
    }
    if crate::claim::foreign_claim(pieces, deploys, ax, az, p.id) {
        events.push(EV_DEPLOY_REFUSED, p.id, REFUSE_D_CLAIM, 0);
        return;
    }
    let supported = match def.placement {
        PLACE_GROUND => level == 0 && ground_ok(seed, haven, pieces, cx, cz),
        PLACE_FOUNDATION => pieces.find(cx, cz, level, LOC_PLANE).is_some(),
        PLACE_ANY => {
            pieces.find(cx, cz, level, LOC_PLANE).is_some()
                || (level == 0 && ground_ok(seed, haven, pieces, cx, cz))
        }
        PLACE_DOORWAY => pieces
            .find(cx, cz, level, loc)
            .is_some_and(|r| bc.pieces[r.row as usize].shape == SHAPE_DOORWAY),
        // A lock's support is the thing it bolts to — a door or a box
        // (`lockable`; locks on boxes, `DOORS.md` §9.8). Anyone in reach
        // may bolt one onto a target that has none — including one
        // somebody else built, which is the reference's claim mechanic
        // (`DOORS.md` §5: the lock is not only who opens it, it is whose
        // door this is).
        PLACE_DOOR => deploys
            .find(cx, cz, level, loc)
            .is_some_and(|r| lockable(dc.defs[r.row as usize].arch)),
        _ => false,
    };
    if !supported {
        let reason = if def.placement == PLACE_GROUND && level == 0 {
            REFUSE_D_TERRAIN
        } else {
            REFUSE_D_SUPPORT
        };
        events.push(EV_DEPLOY_REFUSED, p.id, reason, 0);
        return;
    }
    if def.arch == ARCH_LOCK {
        // The lock's whole placement, and it returns rather than falling
        // through: no `DeployRec` is minted, because a lock is not a
        // deployable — it is a record about one (`lock.rs`).
        if deploys.locks.find(cx, cz, level, loc).is_some() {
            events.push(EV_DEPLOY_REFUSED, p.id, REFUSE_D_HAS_LOCK, 0);
            return;
        }
        if deploys.locks.len() == MAX_LOCKS {
            events.push(EV_DEPLOY_REFUSED, p.id, REFUSE_D_FULL, 0);
            return;
        }
        if !lock::holds(&p.inv, def.item) {
            events.push(EV_DEPLOY_REFUSED, p.id, REFUSE_D_COST, 0);
            return;
        }
        deploys
            .locks
            .insert(LockRec::fresh(cx, cz, level, loc, p.id));
        inv_take(&mut p.inv, def.item, 1);
        // Bolted on, not armed: a fresh lock has no code and lets
        // everyone through until `ACCESS_OP_SET_CODE` gives it one, which
        // is the reference's own sequence. The announcement carries the
        // new `has_lock` bit so the client can prompt for a code — for a
        // box exactly as for a door, since EV_DOOR is addressed and the
        // client's mirror updates whatever record stands there.
        let di = deploys
            .find_index(cx, cz, level, loc)
            .expect("the support check just found the lock's target");
        deploys.entries[di].has_lock = true;
        deploys.entries[di].locked = false;
        announce_door(deploys, di, p.id, events);
        return;
    }
    if def.arch == ARCH_HEARTH {
        // No hearth inside any hearth's radius (own included), and the
        // dense hearth list is a hard cap.
        // No hearth inside any claim, own included — and *claim* is now
        // the base's volume rather than the old circle, so two hearths in
        // one building refuse each other however far apart they stand
        // along it. That is the reference's "one cupboard per building"
        // (`BUILDING.md` §2) falling out of the shape rather than needing
        // a building identity to enforce.
        if crate::claim::any_claim(pieces, deploys, ax, az) {
            events.push(EV_DEPLOY_REFUSED, p.id, REFUSE_D_OVERLAP, 0);
            return;
        }
        if deploys.hearth_count == MAX_HEARTHS {
            events.push(EV_DEPLOY_REFUSED, p.id, REFUSE_D_FULL, 0);
            return;
        }
    }
    if def.arch == ARCH_BAG && deploys.own_bag_count(dc, p.id) >= BAG_CAP {
        events.push(EV_DEPLOY_REFUSED, p.id, REFUSE_D_BAG_CAP, 0);
        return;
    }
    // Checked before the insert for the same reason the hearth cap is: the
    // record below cannot be written into a full store, so the append that
    // follows the insert needs no second guard.
    if holds_items(def.arch) && deploys.boxes.len == MAX_BOXES {
        events.push(EV_DEPLOY_REFUSED, p.id, REFUSE_D_FULL, 0);
        return;
    }
    if inv_count(&p.inv, def.item) < 1 {
        events.push(EV_DEPLOY_REFUSED, p.id, REFUSE_D_COST, 0);
        return;
    }
    let rec = DeployRec {
        cx,
        cz,
        level,
        loc,
        row: row as u8,
        owner: p.id,
        hp: def.hp,
        uh: (tick / UPKEEP_PERIOD_TICKS) as u16,
        open: false,
        // A door is born **bare** (lock v1). lock v0 had it born locked to
        // its placer, which made the door free and the security free; the
        // security is what costs now, and a door nobody has bolted a lock
        // onto is a door anyone in reach may work (`DOORS.md` §9.2).
        has_lock: false,
        locked: false,
        // Wire-only; the store never maintains it (`PieceRec::dmg`).
        dmg: 0,
    };
    if !deploys.insert(rec, tick) {
        events.push(EV_DEPLOY_REFUSED, p.id, REFUSE_D_FULL, 0);
        return;
    }
    if def.arch == ARCH_DOOR {
        // Doors place closed and seal their doorway (door v0).
        pieces.set_door(cx, cz, level, loc, true);
    }
    // A body deploy with a volume becomes movement collision the moment it
    // stands — the same lockstep the shut bit above keeps, one line down
    // (deploy collision v0). Nothing checks whether a player is standing
    // in the new volume, deliberately: the veto-lift in `movement::step`
    // already makes being inside non-absorbing, exactly as a node
    // respawning around a body does.
    if solid_vol(def.arch).is_some() {
        pieces.set_solid(cx, cz, level, Some(def.arch));
    }
    if def.arch == ARCH_HEARTH {
        deploys.hearths[deploys.hearth_count] = HearthRec {
            cx,
            cz,
            level,
            owner: p.id,
            stock: [0; HEARTH_STOCK_ROWS],
            // Placing it joins its crew — the reference's own rule
            // (`BUILDING.md` §1 fact 4), and the reason there is no
            // separate "authorize yourself at your own hearth" step.
            crew: CrewList::of(p.id),
        };
        deploys.hearth_count += 1;
        deploys.hearth_gen += 1;
    }
    // A box, a fire and a furnace are one store: an oven's contents are a
    // box's contents, which is what lets `CONT_BOX` address a campfire
    // and the open/move/sync verbs work on one without a wire field
    // invented for a second kind of container (`oven.rs`).
    if holds_items(def.arch) {
        let n = deploys.boxes.len;
        deploys.boxes.entries[n] = BoxRec {
            cx,
            cz,
            level,
            owner: p.id,
            items: [ItemStack::default(); BOX_SLOTS],
        };
        deploys.boxes.ovens[n] = crate::oven::OvenState {
            arch: def.arch,
            ..Default::default()
        };
        deploys.boxes.len += 1;
    }
    inv_take(&mut p.inv, def.item, 1);
    events.push(
        EV_DEPLOY_PLACED,
        crate::gather::cell_key(cx, cz),
        ((level as u32) << 16) | ((loc as u32) << 8) | row as u32,
        p.id,
    );
}

/// Apply one feed request (`Command::Feed`): move up to `FEED_CHUNK`
/// units per upkeep material from the feeder into the hearth at the
/// address, capped at `STOCK_MAX` per row. Any hearth in reach accepts
/// (a gift is not a grief); EV_STOCK acks the feeder with the address —
/// the wire reads the new stock from the world when it encodes.
pub fn feed(
    dc: &DeployContent,
    deploys: &mut Deploys,
    p: &mut Player,
    cx: u16,
    cz: u16,
    level: u8,
    events: &mut EventQueue,
) {
    let (ax, az) = cell_center(cx, cz);
    let (px, pz) = player_xz(p);
    let (dx, dz) = (ax - px, az - pz);
    let in_reach = dx * dx + dz * dz <= crate::build::BUILD_REACH_M * crate::build::BUILD_REACH_M;
    let Some(h) = deploys.hearths[..deploys.hearth_count]
        .iter()
        .position(|h| h.cx == cx && h.cz == cz && h.level == level)
    else {
        events.push(EV_DEPLOY_REFUSED, p.id, REFUSE_D_HEARTH, 0);
        return;
    };
    if !in_reach {
        events.push(EV_DEPLOY_REFUSED, p.id, REFUSE_D_REACH, 0);
        return;
    }
    for m in 0..dc.mat_count as usize {
        let item = dc.mats[m];
        let room = STOCK_MAX.saturating_sub(deploys.hearths[h].stock[m]);
        let want = FEED_CHUNK.min(room).min(inv_count(&p.inv, item));
        if want > 0 {
            let took = inv_take(&mut p.inv, item, want);
            deploys.hearths[h].stock[m] += took;
        }
    }
    events.push(
        EV_STOCK,
        p.id,
        crate::gather::cell_key(cx, cz),
        level as u32,
    );
}

/// The verb's target at the address, if one of an `accept`ed archetype is
/// there and the player stands within build reach of its cell center.
/// Both refusals are pushed here, so the use and lock paths bounce
/// identically for the same reason. `accept` is `arch_is_door` for the
/// use verb (only a door has a leaf to toggle) and `lockable` for the
/// lock verb (a box carries the same lock — `DOORS.md` §9.8).
#[allow(clippy::too_many_arguments)]
fn door_in_reach(
    dc: &DeployContent,
    deploys: &Deploys,
    p: &Player,
    cx: u16,
    cz: u16,
    level: u8,
    loc: u8,
    accept: fn(u8) -> bool,
    events: &mut EventQueue,
) -> Option<usize> {
    let hit = deploys.entries[..deploys.len]
        .iter()
        .position(|d| d.cx == cx && d.cz == cz && d.level == level && d.loc == loc)
        .filter(|&i| accept(dc.defs[deploys.entries[i].row as usize].arch));
    let Some(i) = hit else {
        events.push(EV_DEPLOY_REFUSED, p.id, REFUSE_D_DOOR, 0);
        return None;
    };
    let (ax, az) = cell_center(cx, cz);
    let (px, pz) = player_xz(p);
    let (dx, dz) = (ax - px, az - pz);
    if dx * dx + dz * dz > crate::build::BUILD_REACH_M * crate::build::BUILD_REACH_M {
        events.push(EV_DEPLOY_REFUSED, p.id, REFUSE_D_REACH, 0);
        return None;
    }
    Some(i)
}

/// EV_DOOR: the deployable's whole leaf-and-lock state after a change,
/// absolute (world.rs documents the packing). One announcement serves the
/// toggle and the lock, so a client never holds half a door — and it
/// serves a **box's** lock bits too (locks on boxes): the event is
/// addressed, the client's mirror updates whatever record stands there,
/// and a box's `open` is simply always false.
fn announce_door(deploys: &Deploys, i: usize, by: u32, events: &mut EventQueue) {
    let d = &deploys.entries[i];
    events.push(
        EV_DOOR,
        crate::gather::cell_key(d.cx, d.cz),
        ((d.level as u32) << 16)
            | ((d.loc as u32) << 8)
            | ((d.has_lock as u32) << 2)
            | ((d.locked as u32) << 1)
            | d.open as u32,
        by,
    );
}

/// Apply one use request (`Command::Use`): toggle the door at the
/// address. Any player within build reach may toggle a door the lock at
/// that address lets them through — which is **every** door with no lock
/// on it, and every door whose lock is unlocked (lock v1). A refusal
/// **knocks**: the shut bit does not move, the sender gets
/// `REFUSE_D_OWNER`, and everyone gets `EV_KNOCK`.
///
/// One predicate, asked once, for open and for close alike — the
/// reference asks its lock on `OnTryToOpen` *and* `OnTryToClose`
/// (`reference/DOORS.md` §1 fact 3) and a toggle gets that for free. It
/// must keep getting it for free: a fast path that skipped the check when
/// closing would let a stranger shut you in.
///
/// The tier is the **door tier** ([`Deploys::lock_passes`] →
/// `Locks::passes`): any grant swings the leaf, which is a GUEST code's
/// whole verb (`DOORS.md` §2.2) — deliberately one tier softer than
/// `pick_up`'s `Locks::passes_full`. The guest press in
/// `a_guest_works_a_locked_door_and_cannot_pocket_it` goes red if the
/// two tiers are ever conflated again.
///
/// The shut bit in the collision index flips in the same call, so the
/// tick that toggles is the tick that blocks (or opens); EV_DOOR
/// announces the new state for the wire.
///
/// Returns **whose leaf this was** when one actually moved — the trust
/// ledger's counterparty (`world.rs`'s `EV_TRUST`). Deliberately the
/// deployable's owner and not the lock's: the thing worked is the door,
/// and a lock is a separate entity that another hand may have bolted on
/// (`reference/DOORS.md` §1). `None` for every refusal, so the row is a
/// record of access *exercised* and never of access asked for — the knock
/// is already the event for that.
#[allow(clippy::too_many_arguments)]
pub fn use_door(
    dc: &DeployContent,
    pieces: &mut Pieces,
    deploys: &mut Deploys,
    p: &mut Player,
    cx: u16,
    cz: u16,
    level: u8,
    loc: u8,
    events: &mut EventQueue,
) -> Option<u32> {
    let i = door_in_reach(dc, deploys, p, cx, cz, level, loc, arch_is_door, events)?;
    if !deploys.lock_passes(cx, cz, level, loc, p.id) {
        // Knocking is the whole reason a refusal here is not silent
        // (`DOORS.md` §4): it is the only channel a locked-out player has
        // to the person inside, and it costs one broadcast. Both go out —
        // the sender still learns *why* nothing swung.
        events.push(
            EV_KNOCK,
            crate::gather::cell_key(cx, cz),
            ((level as u32) << 16) | ((loc as u32) << 8),
            p.id,
        );
        events.push(EV_DEPLOY_REFUSED, p.id, REFUSE_D_OWNER, 0);
        return None;
    }
    let open = !deploys.entries[i].open;
    deploys.entries[i].open = open;
    pieces.set_door(cx, cz, level, loc, !open);
    announce_door(deploys, i, p.id, events);
    Some(deploys.entries[i].owner)
}

/// Apply one lock request (`Command::Access`): run `op` against the code
/// lock at the address (lock v1, `lock.rs` owns the rules).
///
/// Absolute rather than a toggle, for lock v0's reason: two presses
/// racing must agree on the result, not swap it. The leaf never moves —
/// locking an open door leaves it open, and whoever the lock remembers is
/// then the one who can shut it.
///
/// This function is the **seam**, and it is deliberately the only one:
/// `lock.rs` decides, `deploy.rs` spends the item, mirrors the bit onto
/// `DeployRec` and pushes the events. Neither half writes the other's
/// state, so the reason code and the announcement shape exist in exactly
/// one place each.
#[allow(clippy::too_many_arguments)]
pub fn lock_op(
    dc: &DeployContent,
    gc: &GatherContent,
    deploys: &mut Deploys,
    p: &mut Player,
    cx: u16,
    cz: u16,
    level: u8,
    loc: u8,
    op: u8,
    code: u16,
    tick: u64,
    events: &mut EventQueue,
    spill: &mut [ItemStack; INV_SLOTS],
) -> Option<u32> {
    let di = door_in_reach(dc, deploys, p, cx, cz, level, loc, lockable, events)?;
    let Some(li) = deploys.locks.find_index(cx, cz, level, loc) else {
        events.push(EV_DEPLOY_REFUSED, p.id, REFUSE_D_NO_LOCK, 0);
        return None;
    };
    // Read before the op, because the op can take the lock off the door
    // (`Outcome::Removed`) and the owner would then be a field of a record
    // that no longer stands.
    let owner = deploys.locks.entries[li].owner;
    match lock::apply(&mut deploys.locks, li, p.id, op, code, tick) {
        Outcome::Done { relock } => {
            deploys.entries[di].locked = relock;
            announce_door(deploys, di, p.id, events);
        }
        Outcome::Removed => {
            // The lock item comes back to the hand that unbolted it, and
            // the door is anyone's again (`DOORS.md` §7 verb 8). A full
            // inventory used to lose it, and this comment used to argue
            // that was correct because every other give here did the same
            // — which stopped being true the moment the other gives took
            // the spill. It falls at the unbolter's feet now, so a player
            // carrying a full load can still take a lock off *and* keep it.
            if let Some(r) = lock_row(dc) {
                let item = dc.defs[r].item;
                lock::give_back(
                    &mut p.inv,
                    spill,
                    item,
                    gc.stack_max_of(item),
                    gc.cond_max_of(item),
                );
            }
            deploys.entries[di].has_lock = false;
            deploys.entries[di].locked = false;
            announce_door(deploys, di, p.id, events);
        }
        Outcome::Authorized { grant } => {
            events.push(
                EV_AUTH,
                crate::gather::cell_key(cx, cz),
                ((level as u32) << 16) | ((loc as u32) << 8) | grant as u32,
                p.id,
            );
            // The one outcome here that is a *trust* act: a hand this lock
            // did not know now stands on its list, and the owner may or
            // may not have been there to see it (`world.rs`'s `EV_TRUST`).
            return Some(owner);
        }
        Outcome::Wrong { shock, shut } => {
            // The funnel, **unreduced**: the shock's whole job is to cost
            // tries, and an armored raider immune to it is a keypad with
            // no ladder. `lock::shock_amount` still owns the floor at 1 hp
            // — it is the door's rule, not the funnel's.
            let took = lock::shock_amount(p.hp, shock);
            crate::combat::hurt_unreduced(p, took);
            // Absolute, like every other health reading (`EV_HEALTH`'s own
            // doc): a client that misses this one hears the whole truth
            // from the next.
            events.push(EV_HEALTH, p.id, p.hp as u32, p.hp_max as u32);
            let reason = if shut {
                REFUSE_D_LOCKOUT
            } else {
                REFUSE_D_CODE
            };
            events.push(EV_DEPLOY_REFUSED, p.id, reason, 0);
        }
        Outcome::LockedOut => events.push(EV_DEPLOY_REFUSED, p.id, REFUSE_D_LOCKOUT, 0),
        Outcome::Denied => events.push(EV_DEPLOY_REFUSED, p.id, REFUSE_D_OWNER, 0),
        Outcome::ListFull => events.push(EV_DEPLOY_REFUSED, p.id, REFUSE_D_AUTH_FULL, 0),
        Outcome::BadOp => events.push(EV_DEPLOY_REFUSED, p.id, REFUSE_D_KIND, 0),
    }
    None
}

/// Take a deployable back up and return its item — **pickup v1**
/// (`reference/BUILDING.md` §7 verb 11, and `DOORS.md` §7 verb 9, which
/// are the same verb seen from two documents).
///
/// **Deliberately not window-gated, where the piece verb is.** The
/// reference lets an authorized player pick a deployable up inside
/// privilege at any time and only time-boxes *building blocks*, and the
/// asymmetry is right: a box is furniture and a wall is a base. The
/// consequence worth stating is the one `DOORS.md` §5 already states —
/// on unclaimed ground **anyone in reach may take it**, which is what
/// makes a hearth worth placing.
///
/// A deployable with a lock on it is the one extra check: the lock is the
/// thing saying whose door — or whose box — this is, so a hand it does
/// not know cannot lift the thing out from under it. Without that, every
/// code lock in the game would be defeated by picking up what it is
/// bolted to. The tier is **full rights**, not any grant
/// (`Locks::passes_full`): a pickup pockets the lock, and taking the lock
/// off is on the guest tier's "nothing else" side (`DOORS.md` §2.2,
/// Devblog 149's list) — so a guest-code holder is refused here exactly
/// as `ACCESS_OP_TAKE` refuses them. An unlocked one stays anyone's,
/// which is demolish v1's landed rule.
///
/// `spill` catches both gives — the deployable and a lock that came up
/// with it — for a pack that has no room (`NOW.md` §0sp2, 2026-08-14). The
/// fall-point is the picker's feet and `build::demolish`'s doc has the
/// whole argument for why the deployable's own cell is not a rival answer:
/// this verb refuses beyond `BUILD_REACH_M` too, which is the merge radius.
#[allow(clippy::too_many_arguments)]
pub fn pick_up(
    dc: &DeployContent,
    gc: &GatherContent,
    pieces: &mut Pieces,
    deploys: &mut Deploys,
    p: &mut Player,
    cx: u16,
    cz: u16,
    level: u8,
    loc: u8,
    events: &mut EventQueue,
    spill: &mut [ItemStack; INV_SLOTS],
) {
    let Some(i) = deploys.find_index(cx, cz, level, loc) else {
        events.push(EV_DEPLOY_REFUSED, p.id, REFUSE_D_SPOT, 0);
        return;
    };
    let (ax, az) = cell_center(cx, cz);
    let (px, pz) = player_xz(p);
    let (dx, dz) = (ax - px, az - pz);
    if dx * dx + dz * dz > crate::build::BUILD_REACH_M * crate::build::BUILD_REACH_M {
        events.push(EV_DEPLOY_REFUSED, p.id, REFUSE_D_REACH, 0);
        return;
    }
    if crate::claim::foreign_claim(pieces, deploys, ax, az, p.id) {
        events.push(EV_DEPLOY_REFUSED, p.id, REFUSE_D_CLAIM, 0);
        return;
    }
    if !deploys.locks.passes_full(cx, cz, level, loc, p.id) {
        events.push(EV_DEPLOY_REFUSED, p.id, REFUSE_D_OWNER, 0);
        return;
    }
    let rec = deploys.entries[i];
    let def = dc.defs[rec.row as usize];
    // A container comes up empty or not at all.
    if def.arch == ARCH_BOX
        && deploys
            .box_index(box_key(cx, cz, level))
            .is_some_and(|b| !deploys.boxes.entries[b].is_empty())
    {
        events.push(EV_DEPLOY_REFUSED, p.id, REFUSE_D_NOT_EMPTY, 0);
        return;
    }
    // ...and a hearth with stock in it would take the stock with it.
    if def.arch == ARCH_HEARTH
        && deploys.hearths[..deploys.hearth_count]
            .iter()
            .any(|h| h.cx == cx && h.cz == cz && h.level == level && h.stock.iter().any(|&s| s > 0))
    {
        events.push(EV_DEPLOY_REFUSED, p.id, REFUSE_D_NOT_EMPTY, 0);
        return;
    }
    // A door's lock comes up with it, as a second item: it is a separate
    // thing bolted on (`DOORS.md` §1 fact 1), so it is separately
    // returned rather than destroyed with the frame.
    if deploys.locks.find(cx, cz, level, loc).is_some() {
        if let Some(r) = lock_row(dc) {
            let item = dc.defs[r].item;
            lock::give_back(
                &mut p.inv,
                spill,
                item,
                gc.stack_max_of(item),
                gc.cond_max_of(item),
            );
        }
    }
    crate::gather::inv_add_spilling(
        &mut p.inv,
        spill,
        def.item,
        1,
        gc.stack_max_of(def.item),
        gc.cond_max_of(def.item),
    );
    drop_deploy(dc, pieces, deploys, i, events);
}

/// Apply one crew op to the hearth at the address (hearth crew v1,
/// `reference/BUILDING.md` §9.1). `Command::Access` routes here when the
/// address holds a hearth and to `lock_op` when it holds a door — one
/// verb, because "who may do this here" is one question and the answer
/// lives in a `Roster` either way.
///
/// The reference's own three (`BUILDING.md` §2): press to add yourself,
/// press again to remove yourself, hold for clear. **Self-service is the
/// whole model** — you authorize *yourself* at a hearth whose crew let you
/// reach it, which is why there is no id on the wire and nothing to forge:
/// the only player a crew op can name is its sender.
///
/// Returns **whose hearth this was** when an op actually landed — the same
/// counterparty `use_door` returns and for the same ledger (`world.rs`'s
/// `EV_TRUST`). The op worth having it for is the clear: only a crew
/// member may run it and it evicts everyone else, so a crew member who
/// clears a hearth they did not place locks its owner out of their own
/// base — which is the shape of betrayal this record exists to be able to
/// count, and the shape whose whole meaning is whether the owner was
/// there.
#[allow(clippy::too_many_arguments)]
pub fn crew_op(
    deploys: &mut Deploys,
    p: &Player,
    cx: u16,
    cz: u16,
    level: u8,
    op: u8,
    events: &mut EventQueue,
) -> Option<u32> {
    let Some(h) = deploys.hearths[..deploys.hearth_count]
        .iter()
        .position(|hr| hr.cx == cx && hr.cz == cz && hr.level == level)
    else {
        events.push(EV_DEPLOY_REFUSED, p.id, REFUSE_D_HEARTH, 0);
        return None;
    };
    let (ax, az) = cell_center(cx, cz);
    let (px, pz) = player_xz(p);
    let (dx, dz) = (ax - px, az - pz);
    if dx * dx + dz * dz > crate::build::BUILD_REACH_M * crate::build::BUILD_REACH_M {
        events.push(EV_DEPLOY_REFUSED, p.id, REFUSE_D_REACH, 0);
        return None;
    }
    let crew = &mut deploys.hearths[h].crew;
    // Whether the crew's membership actually moved. The trust row rides
    // this and not the op's success, because one op succeeds while
    // changing nothing: a `CREW_LEAVE` is deliberately ungated — refusing
    // it would tell a stranger whether the crew knew them — so a hand that
    // was never on the list presses it, `remove` returns false, and the
    // announcement below still goes out. Returning the owner there would
    // log a betrayal for a button that did nothing, in a record whose
    // whole value is that its rows mean something.
    let moved;
    match op {
        // **Anyone in reach may join an empty-crewed hearth, and only the
        // crew may join a crewed one.** The first half cannot happen —
        // placing joins the crew, so a live hearth always has at least one
        // member — but stating it here is what makes the rule readable as
        // the same one the lock keeps: an unclaimed thing is anyone's.
        ACCESS_OP_CREW_JOIN => {
            if !crew.is_empty() && !crew.contains(p.id) {
                events.push(EV_DEPLOY_REFUSED, p.id, REFUSE_D_OWNER, 0);
                return None;
            }
            // `Already` is a re-press by a member: the list is unchanged,
            // so nothing was newly entrusted.
            match crew.add(p.id) {
                Added::Full => {
                    events.push(EV_DEPLOY_REFUSED, p.id, REFUSE_D_AUTH_FULL, 0);
                    return None;
                }
                added => moved = added == Added::New,
            }
        }
        ACCESS_OP_CREW_LEAVE => {
            // Leaving is not gated: a hand the crew does not know is
            // already not on it, and a refusal there would tell a
            // stranger whether they were. `remove` returning false is
            // simply nothing happening.
            moved = crew.remove(p.id);
        }
        ACCESS_OP_CREW_CLEAR => {
            if !crew.contains(p.id) {
                events.push(EV_DEPLOY_REFUSED, p.id, REFUSE_D_OWNER, 0);
                return None;
            }
            // Clear to the clearer, one operation. A bare `clear()` would
            // leave the hearth crewless for the instant between two
            // statements, and an empty crew is a hearth anyone may join.
            crew.reset_to(p.id);
            moved = true;
        }
        _ => {
            events.push(EV_DEPLOY_REFUSED, p.id, REFUSE_D_KIND, 0);
            return None;
        }
    }
    // The sender's own standing afterwards, absolute, to the sender only —
    // `EV_AUTH`'s existing shape, reused rather than duplicated. A crew
    // grant is full or nothing: there is no guest tier for building.
    let grant = if deploys.hearths[h].crew.contains(p.id) {
        lock::GRANT_FULL
    } else {
        lock::GRANT_NONE
    };
    events.push(
        EV_AUTH,
        crate::gather::cell_key(cx, cz),
        ((level as u32) << 16) | ((LOC_PLANE as u32) << 8) | grant as u32,
        p.id,
    );
    moved.then_some(deploys.hearths[h].owner)
}

/// The baked row of the code lock, if the loaded content has one. A table
/// with no lock row is inert content and the take verb simply returns
/// nothing — the same posture every other verb here takes toward the
/// `EMPTY` default (a shard booted without locks plays the game it played
/// before, wall 7).
fn lock_row(dc: &DeployContent) -> Option<usize> {
    dc.defs[..dc.def_count as usize]
        .iter()
        .position(|d| d.arch == ARCH_LOCK)
}

/// Per-period charge for one cost row: `ceil(cost × pct / 100 / 24)`.
fn charge_of(cost: u16, pct: u16) -> u32 {
    let num = cost as u32 * pct as u32;
    num.div_ceil(100 * PERIODS_PER_DAY)
}

/// Per-period decay for a max hp at a rate: `max(1, maxhp × pct / 100)`.
///
/// The floor is what stops a 1 % rate on a 50 hp piece rounding to zero
/// and making it immortal — a decay that never subtracts is a decay
/// that is off, and off is a thing content should have to *say*
/// (`upkeep_pct_per_day = 0`) rather than stumble into.
fn decay_at(max_hp: u16, pct: u32) -> u16 {
    ((max_hp as u32 * pct) / 100).max(1) as u16
}

/// The rate a **piece** of this material rots at, from the baked ladder.
/// Content that priced no ladder falls back to the flat rate, so a shard
/// on an older `balance.toml` plays exactly the game it played before —
/// a new table may add a rule, never silently change one (wall 7).
fn piece_decay_pct(dc: &DeployContent, material: u8) -> u32 {
    match dc.decay_pct.get(material as usize) {
        Some(&p) if p > 0 => p as u32,
        _ => DECAY_PCT_PER_PERIOD,
    }
}

/// Remove the piece at store index `i` and the deployable standing at the
/// exact same address (the door in the doorway, the box on the floor),
/// broadcasting both removals. **The one removal path**: decay reaches it
/// through the sweep, a raid through `damage_piece`, and neither can grow
/// a cascade the other lacks.
///
/// This is the single removal, not the structural one: a caller that took
/// the piece out of the world follows it with `build::collapse_from` so
/// what rested on it comes down too. `collapse_from` itself calls this and
/// must not, which is why the two are separate functions.
pub(crate) fn drop_piece(
    dc: &DeployContent,
    pieces: &mut Pieces,
    deploys: &mut Deploys,
    i: usize,
    shape: u8,
    events: &mut EventQueue,
) {
    let rec = pieces.entries()[i];
    pieces.remove_at(i, shape);
    events.push(
        EV_PIECE_REMOVED,
        crate::gather::cell_key(rec.cx, rec.cz),
        ((rec.level as u32) << 16) | ((rec.loc as u32) << 8) | rec.row as u32,
        0,
    );
    if let Some(di) = deploys.entries[..deploys.len]
        .iter()
        .position(|d| d.cx == rec.cx && d.cz == rec.cz && d.level == rec.level && d.loc == rec.loc)
    {
        drop_deploy(dc, pieces, deploys, di, events);
    }
}

/// Remove the deployable at store index `di`, unsealing its doorway if it
/// was a door, and broadcast the removal. The other half of the one
/// removal path.
fn drop_deploy(
    dc: &DeployContent,
    pieces: &mut Pieces,
    deploys: &mut Deploys,
    di: usize,
    events: &mut EventQueue,
) {
    let rec = deploys.entries[di];
    deploys.remove_at(di, dc);
    if dc.defs[rec.row as usize].arch == ARCH_DOOR {
        pieces.set_door(rec.cx, rec.cz, rec.level, rec.loc, false);
    }
    // The blocked volume leaves with the record — the clear half of the
    // lockstep `place_deploy` set. Here rather than at each caller for
    // `remove_at_address`'s reason one block down: this is the one
    // removal path, and a solid bit outliving its furnace would wall off
    // an empty cell forever.
    if solid_vol(dc.defs[rec.row as usize].arch).is_some() {
        pieces.set_solid(rec.cx, rec.cz, rec.level, None);
    }
    if lockable(dc.defs[rec.row as usize].arch) {
        // A lock dies with what it is bolted to (`DOORS.md` §2.2) — a
        // box's exactly as a door's. Here rather than at each caller,
        // because this is the one removal path — decay, a raid swing and
        // a collapsing doorway all arrive through it, and a lock left
        // behind at a dead address would silently refuse the next thing
        // built there.
        deploys
            .locks
            .remove_at_address(rec.cx, rec.cz, rec.level, rec.loc);
    }
    events.push(
        EV_DEPLOY_REMOVED,
        crate::gather::cell_key(rec.cx, rec.cz),
        ((rec.level as u32) << 16) | ((rec.loc as u32) << 8) | rec.row as u32,
        0,
    );
}

/// Take `amount` hp off the piece at store index `i` — the raid verb's
/// write (combat.rs picks the target, this owns the store). Broadcasts
/// `EV_STRUCT_HIT` with the hp left, then removes through `drop_piece` at
/// zero. Returns true when the piece fell.
///
/// hp is otherwise sim-only (build.rs) — a raid is the one case where the
/// number has to cross, because a wall that shows no progress under thirty
/// swings reads as an invulnerable wall.
#[allow(clippy::too_many_arguments)]
pub fn damage_piece(
    dc: &DeployContent,
    bc: &BuildContent,
    pieces: &mut Pieces,
    deploys: &mut Deploys,
    i: usize,
    amount: u16,
    budget: &mut usize,
    events: &mut EventQueue,
) -> bool {
    let rec = pieces.entries()[i];
    let mut left = rec.hp.saturating_sub(amount);
    if left == 0 && *budget == 0 {
        // The tick's removal budget is spent (limits.rs
        // `MAX_REMOVALS_PER_TICK`), so the wall stops at its last hp and
        // the next swing takes it. Deferring here rather than after the
        // subtraction is what keeps a standing piece's hp at 1 or more:
        // parking a 0-hp piece in the store would leave a wall that no
        // sweep removes and every swing re-reports as already broken.
        // The raider is told the truth — this hit reads as 0 dealt.
        left = 1;
    }
    let dealt = rec.hp - left;
    events.push(
        EV_STRUCT_HIT,
        crate::gather::cell_key(rec.cx, rec.cz),
        ((rec.level as u32) << 16) | ((rec.loc as u32) << 8) | rec.row as u32,
        ((dealt as u32) << 16) | left as u32,
    );
    if left == 0 {
        drop_piece(
            dc,
            pieces,
            deploys,
            i,
            bc.pieces[rec.row as usize].shape,
            events,
        );
        *budget -= 1;
        // Take the legs out and the base comes down: everything that
        // rested on this address is re-checked against the same support
        // rule that let it be placed (build.rs).
        crate::build::collapse_from(
            dc,
            bc,
            pieces,
            deploys,
            (rec.cx, rec.cz, rec.level, rec.loc),
            budget,
            events,
        );
        return true;
    }
    pieces.set_hp(i, left);
    false
}

/// Take `amount` hp off the deployable at store index `di`. The deployable
/// half of `damage_piece`, `STRUCT_DEPLOY_BIT` set on the wire so a client
/// knows which store the address names.
pub fn damage_deploy(
    dc: &DeployContent,
    pieces: &mut Pieces,
    deploys: &mut Deploys,
    di: usize,
    amount: u16,
    events: &mut EventQueue,
) -> bool {
    let rec = deploys.entries[di];
    let left = rec.hp.saturating_sub(amount);
    let dealt = rec.hp - left;
    events.push(
        EV_STRUCT_HIT,
        crate::gather::cell_key(rec.cx, rec.cz),
        STRUCT_DEPLOY_BIT | ((rec.level as u32) << 16) | ((rec.loc as u32) << 8) | rec.row as u32,
        ((dealt as u32) << 16) | left as u32,
    );
    if left == 0 {
        drop_deploy(dc, pieces, deploys, di, events);
        return true;
    }
    deploys.entries[di].hp = left;
    false
}

/// One tick of the upkeep/decay sweep: advance each store's cursor by
/// `UPKEEP_SWEEP_PER_TICK` entries, processing due periods per entry
/// (charge from a covering hearth or decay; removal at 0 hp). Bounded
/// work, deterministic order (cursor + list order). Piece removal also
/// removes the deployable at the exact same address (the door in the
/// decayed doorway, the box on the decayed floor).
#[allow(clippy::too_many_arguments)]
pub fn upkeep_sweep(
    dc: &DeployContent,
    bc: &BuildContent,
    pieces: &mut Pieces,
    deploys: &mut Deploys,
    tick: u64,
    piece_cursor: &mut u32,
    deploy_cursor: &mut u32,
    budget: &mut usize,
    events: &mut EventQueue,
) {
    if dc.mat_count == 0 {
        return; // no upkeep materials priced: decay is off (inert table)
    }
    // The coverage cache every question below reads, refreshed at this
    // one fixed point in the tick — after the commands, before any query,
    // whether or not any entry is due. `claim::ClaimCache`'s doc carries
    // the determinism argument; what matters here is the order: no
    // coverage question is ever asked of a cache the tick's commands have
    // not been folded into.
    deploys.refresh_claims(pieces);
    let h_now = (tick / UPKEEP_PERIOD_TICKS) as u16;

    // --- pieces ---------------------------------------------------------
    let mut visits = 0usize;
    while visits < UPKEEP_SWEEP_PER_TICK && !pieces.is_empty() {
        visits += 1;
        let i = (*piece_cursor as usize) % pieces.len();
        *piece_cursor = ((i + 1) % pieces.len()) as u32;
        let rec = pieces.entries()[i];
        if rec.uh >= h_now {
            continue;
        }
        let def = bc.pieces[rec.row as usize];
        let (x, z) = cell_center(rec.cx, rec.cz);
        // Which hearths cover this piece, in hearth-list order — asked of
        // the cached claim volume (the base's own shape, `claim.rs`)
        // rather than a circle, which is the whole change: the far end of
        // a long base is covered because the structure reaches it, and a
        // detached shack inside the old radius is not. Asked once per
        // visit, outside the hour steps below, because the steps spend
        // stock and never move structure.
        let mut cover = [0u16; MAX_HEARTHS];
        let mut cover_n = 0usize;
        for hi in 0..deploys.hearth_count {
            if deploys.hearth_covers(hi, x, z) {
                cover[cover_n] = hi as u16;
                cover_n += 1;
            }
        }
        let mut hp = rec.hp;
        let mut uh = rec.uh;
        let mut removed = false;
        let mut steps = 0u32;
        while uh < h_now && steps < SWEEP_CATCHUP_MAX {
            steps += 1;
            uh += 1;
            // **Per material, not all-or-nothing** (upkeep/decay v1,
            // `reference/BUILDING.md` §4). Each cost row is charged to the
            // first covering hearth in list order that can cover *that
            // row*; the piece is protected only if **every** row found a
            // payer.
            //
            // The old rule wanted one hearth to cover the whole charge, so
            // a hearth holding stone but no wood protected nothing at all
            // — half a stock did half of nothing. The reference's is the
            // better sentence and it is not more code: *if your stone runs
            // out, only the stone parts of your base lose health.*
            //
            // Rows are charged as they are found rather than after a
            // whole-piece check, so a piece that pays three rows and
            // misses the fourth has still spent the three. That is the
            // honest reading of a partial payment: the materials went into
            // the base, and the base still rots for want of the one that
            // did not.
            // **A hearth has to cover it at all**, before any row is
            // priced. Without this the per-row loop calls a piece paid
            // when it simply costs nothing, and an unpriced piece in open
            // ground would never rot — which is the old rule's one
            // property worth keeping: no hearth, no protection.
            //
            // **And twig is never paid for, at any hearth** (twig v0,
            // `reference/BUILDING.md` §7b.4). A scaffold is a draft: it
            // costs no upkeep, so a base under construction does not
            // quietly drain the stock that keeps the finished half
            // standing, and no stock can protect it either — starting
            // `all_paid` false here skips the charge loop entirely and
            // sends it straight to the decay step, where the ladder's
            // 100 %/period is waiting. That is the whole difference
            // between a draft you re-lay for 50 wood and a cheap
            // permanent base nobody upgrades.
            let mut all_paid = cover_n > 0 && def.material != crate::build::MAT_TWIG;
            for m in 0..(if all_paid { dc.mat_count as usize } else { 0 }) {
                let due: u32 = def
                    .costs
                    .iter()
                    .take(def.n_costs as usize)
                    .filter(|&&(item, _)| item == dc.mats[m])
                    .map(|&(_, cost)| charge_of(cost, dc.upkeep_pct_per_day))
                    .sum();
                if due == 0 {
                    continue;
                }
                let payer = cover[..cover_n]
                    .iter()
                    .position(|&hi| deploys.hearths[hi as usize].stock[m] >= due);
                match payer {
                    Some(ci) => deploys.hearths[cover[ci] as usize].stock[m] -= due,
                    None => all_paid = false,
                }
            }
            if !all_paid {
                let d = decay_at(def.hp, piece_decay_pct(dc, def.material));
                if hp <= d {
                    hp = 0;
                    removed = true;
                    break;
                }
                hp -= d;
            }
        }
        if steps >= SWEEP_CATCHUP_MAX {
            uh = h_now; // bounded catch-up: the missed hours are forgiven
        }
        if removed {
            if *budget == 0 {
                // The tick's removal budget is spent (limits.rs
                // `MAX_REMOVALS_PER_TICK`). Defer *before* the removal and
                // before `set_upkeep`, so this entry keeps the hp and the
                // upkeep hour it came in with and the whole computation
                // above is simply redone next tick. Rewind the cursor onto
                // this entry rather than past it: the sweep advances by
                // visiting, and a deferred entry has not been served.
                //
                // `break`, not `return`: only the piece half spends this
                // budget. Deployable decay below removes at most one
                // record per visit with no cascade under it, so it is
                // already bounded by `UPKEEP_SWEEP_PER_TICK` and stopping
                // it here would be a second, unstated cap.
                *piece_cursor = i as u32;
                break;
            }
            // Same removal path a raid takes — cascade included, and the
            // structural collapse with it: a floor whose wall decayed out
            // from under it falls exactly as one a raider broke would.
            drop_piece(dc, pieces, deploys, i, def.shape, events);
            *budget -= 1;
            crate::build::collapse_from(
                dc,
                bc,
                pieces,
                deploys,
                (rec.cx, rec.cz, rec.level, rec.loc),
                budget,
                events,
            );
            *piece_cursor = (i as u32).min(pieces.len().saturating_sub(1) as u32);
        } else {
            pieces.set_upkeep(i, hp, uh);
        }
    }

    // --- deployables ----------------------------------------------------
    let mut visits = 0usize;
    while visits < UPKEEP_SWEEP_PER_TICK && deploys.len > 0 {
        visits += 1;
        let i = (*deploy_cursor as usize) % deploys.len;
        *deploy_cursor = ((i + 1) % deploys.len) as u32;
        let rec = deploys.entries[i];
        if rec.uh >= h_now {
            continue;
        }
        let def = dc.defs[rec.row as usize];
        let (x, z) = cell_center(rec.cx, rec.cz);
        let mut hp = rec.hp;
        let mut uh = rec.uh;
        let mut removed = false;
        let mut steps = 0u32;
        while uh < h_now && steps < SWEEP_CATCHUP_MAX {
            steps += 1;
            uh += 1;
            // Covered by any stocked hearth ⇒ free; uncovered ⇒ decay.
            // (An empty hearth covers nothing, itself included.)
            if deploys.covering_hearth(x, z).is_none() {
                // The flat rate: a deployable has no build material, so
                // the ladder has nothing to key on (`piece_decay_pct`).
                let d = decay_at(def.hp, DECAY_PCT_PER_PERIOD);
                if hp <= d {
                    hp = 0;
                    removed = true;
                    break;
                }
                hp -= d;
            }
        }
        if steps >= SWEEP_CATCHUP_MAX {
            uh = h_now;
        }
        if removed {
            drop_deploy(dc, pieces, deploys, i, events);
            *deploy_cursor = (i as u32).min(deploys.len.saturating_sub(1) as u32);
        } else {
            deploys.entries[i].hp = hp;
            deploys.entries[i].uh = uh;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build::{BuildContent, Pieces, LOC_PLANE, REFUSE_B_SPOT, REFUSE_B_UNPRICED};
    use crate::gather::ItemStack;
    use crate::limits::MAX_REMOVALS_PER_TICK;

    /// One tick's structural removal budget, as `World::tick` hands it
    /// out — these fixtures never approach it (build.rs owns the tests
    /// that do).
    fn tick_budget() -> usize {
        MAX_REMOVALS_PER_TICK
    }

    use crate::movement::Body;
    use crate::world::{EventQueue, Player};

    const SEED: u64 = 20260731;

    /// The solved authored sites for `SEED`, memoized.
    ///
    /// `terrain::haven` is a few thousand `height` taps and these cases call
    /// the carved-ground path from nearly every assertion, so resolving it
    /// once per suite is the difference between a fast test and a slow one.
    /// It is a pure function of the seed, so caching it cannot change a result.
    fn hv() -> &'static crate::terrain::Haven {
        static HV: std::sync::OnceLock<crate::terrain::Haven> = std::sync::OnceLock::new();
        HV.get_or_init(|| crate::terrain::haven(SEED))
    }
    const CX: u16 = 341;
    const CZ: u16 = 341;

    fn player_at_cell(cx: u16, cz: u16, items: &[(u16, u16)]) -> Player {
        let mut p = Player {
            id: 7,
            active: true,
            body: Body::at(
                SEED,
                hv(),
                (cx as f32 + 0.5) * crate::build::BUILD_CELL_M,
                (cz as f32 + 0.5) * crate::build::BUILD_CELL_M,
            ),
            ..Player::default()
        };
        for (i, &(item, count)) in items.iter().enumerate() {
            p.inv[i] = ItemStack {
                item,
                count,
                cond: 0,
            };
        }
        p
    }

    fn last(ev: &EventQueue) -> (u8, u32, u32, u32) {
        let e = ev.entries()[ev.len() - 1];
        (e.code, e.a, e.b, e.c)
    }

    /// A foundation at (cx, cz) so foundation-class placement has support.
    fn founded(bc: &BuildContent, pieces: &mut Pieces, p: &mut Player, cx: u16, cz: u16) {
        let mut ev = EventQueue::default();
        crate::build::place(
            SEED,
            hv(),
            bc,
            &Deploys::new(),
            pieces,
            p,
            0,
            0,
            cx,
            cz,
            0,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(last(&ev).0, crate::world::EV_PIECE_PLACED);
    }

    /// The probe fixture's **stone** foundation: row 0's rung, and the
    /// row every fixture here stands on when the question is upkeep.
    /// Named once so the hp assertions and the two builders cannot drift
    /// apart (`build::BuildContent::probe_fixture`).
    const GRADED_FOUNDATION: usize = 5;

    /// `founded`, then committed to stone — a foundation the upkeep sweep
    /// can actually charge. Twig is never upkept and therefore never
    /// protected (twig v0), so a test asking *what a hearth pays for*
    /// cannot ask it of a scaffold: the answer is "nothing" before the
    /// claim is consulted. Row 5 is row 0's stone rung and carries row 0's
    /// cost exactly, so every charge in this module's arithmetic comments
    /// still reads 5 × item 0.
    fn founded_graded(bc: &BuildContent, pieces: &mut Pieces, p: &mut Player, cx: u16, cz: u16) {
        founded(bc, pieces, p, cx, cz);
        let mut ev = EventQueue::default();
        crate::build::upgrade(
            bc,
            &Deploys::new(),
            pieces,
            p,
            cx,
            cz,
            0,
            LOC_PLANE,
            crate::build::MAT_STONE,
            &mut ev,
        );
        assert_eq!(
            pieces.find(cx, cz, 0, LOC_PLANE).unwrap().row,
            GRADED_FOUNDATION as u8,
            "the fixture foundation never reached its stone rung"
        );
    }

    #[test]
    fn placement_classes_enforce_their_support() {
        let dc = DeployContent::probe_fixture();
        let bc = BuildContent::probe_fixture();
        let mut pieces = Pieces::new();
        let mut deploys = Deploys::new();
        let mut ev = EventQueue::default();
        let mut p = player_at_cell(CX, CZ, &[(0, 99), (1, 99), (2, 5), (3, 5), (4, 5), (5, 5)]);

        // Hearth (foundation class) on bare terrain refuses.
        place_deploy(
            SEED,
            hv(),
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut p,
            0,
            0,
            CX,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(last(&ev).2, REFUSE_D_SUPPORT);

        // Bag (ground class) on bare terrain places and pays its item.
        place_deploy(
            SEED,
            hv(),
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut p,
            0,
            3,
            CX,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(last(&ev).0, crate::world::EV_DEPLOY_PLACED);
        assert_eq!(inv_count(&p.inv, 5), 4, "bag item consumed");

        // Occupied address refuses.
        place_deploy(
            SEED,
            hv(),
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut p,
            0,
            3,
            CX,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(last(&ev).2, REFUSE_D_SPOT);

        // Foundation next door, hearth on it places; ground bag on the
        // foundation cell refuses (ground means terrain).
        founded(&bc, &mut pieces, &mut p, CX + 1, CZ);
        place_deploy(
            SEED,
            hv(),
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut p,
            0,
            0,
            CX + 1,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(last(&ev).0, crate::world::EV_DEPLOY_PLACED);
        assert_eq!(deploys.hearths().len(), 1);
        place_deploy(
            SEED,
            hv(),
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut p,
            0,
            3,
            CX + 1,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(last(&ev).2, REFUSE_D_SPOT, "hearth holds the address");

        // Door needs a doorway edge piece: none ⇒ support refusal; a wall
        // there is still not a doorway.
        place_deploy(
            SEED,
            hv(),
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut p,
            0,
            2,
            CX + 1,
            CZ,
            0,
            LOC_EDGE_XLO,
            &mut ev,
        );
        assert_eq!(last(&ev).2, REFUSE_D_SUPPORT);
        crate::build::place(
            SEED,
            hv(),
            &bc,
            &deploys,
            &mut pieces,
            &mut p,
            0,
            1,
            CX + 1,
            CZ,
            0,
            LOC_EDGE_XLO,
            &mut ev,
        );
        assert_eq!(last(&ev).0, crate::world::EV_PIECE_PLACED);
        place_deploy(
            SEED,
            hv(),
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut p,
            0,
            2,
            CX + 1,
            CZ,
            0,
            LOC_EDGE_XLO,
            &mut ev,
        );
        assert_eq!(last(&ev).2, REFUSE_D_SUPPORT, "a wall is not a doorway");

        // Bad row, wrong loc, empty pocket.
        place_deploy(
            SEED,
            hv(),
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut p,
            0,
            9,
            CX,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(last(&ev).2, REFUSE_D_KIND);
        place_deploy(
            SEED,
            hv(),
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut p,
            0,
            3,
            CX,
            CZ,
            0,
            LOC_EDGE_XLO,
            &mut ev,
        );
        assert_eq!(last(&ev).2, REFUSE_D_SPOT);
        let mut poor = player_at_cell(CX + 2, CZ, &[]);
        place_deploy(
            SEED,
            hv(),
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut poor,
            0,
            3,
            CX + 2,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(last(&ev).2, REFUSE_D_COST);
    }

    #[test]
    fn hearth_claims_and_overlap_refuse() {
        let dc = DeployContent::probe_fixture();
        let bc = BuildContent::probe_fixture();
        let mut pieces = Pieces::new();
        let mut deploys = Deploys::new();
        let mut ev = EventQueue::default();
        let mut own = player_at_cell(CX, CZ, &[(0, 99), (1, 99), (2, 9)]);
        founded(&bc, &mut pieces, &mut own, CX, CZ);
        place_deploy(
            SEED,
            hv(),
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut own,
            0,
            0,
            CX,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(last(&ev).0, crate::world::EV_DEPLOY_PLACED);

        // A stranger inside the radius: pieces refuse with the claim
        // reason, deploys too; outside the radius both work.
        let mut foe = player_at_cell(CX + 2, CZ, &[(0, 99), (1, 99), (5, 5)]);
        foe.id = 8;
        crate::build::place(
            SEED,
            hv(),
            &bc,
            &deploys,
            &mut pieces,
            &mut foe,
            0,
            0,
            CX + 2,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(
            last(&ev),
            (
                crate::world::EV_BUILD_REFUSED,
                8,
                crate::build::REFUSE_B_CLAIM,
                0
            )
        );
        place_deploy(
            SEED,
            hv(),
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut foe,
            0,
            3,
            CX + 2,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(last(&ev).2, REFUSE_D_CLAIM);
        // ~30 m away (10 cells): outside the 24 m radius.
        let far = CX + 10;
        let mut foe_far = player_at_cell(far, CZ, &[(0, 99), (1, 99), (5, 5)]);
        foe_far.id = 8;
        crate::build::place(
            SEED,
            hv(),
            &bc,
            &deploys,
            &mut pieces,
            &mut foe_far,
            0,
            0,
            far,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(last(&ev).0, crate::world::EV_PIECE_PLACED);

        // The owner may keep building inside their own radius…
        crate::build::place(
            SEED,
            hv(),
            &bc,
            &deploys,
            &mut pieces,
            &mut own,
            0,
            0,
            CX + 1,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(last(&ev).0, crate::world::EV_PIECE_PLACED);
        // …but not stack a second hearth inside the first's radius.
        place_deploy(
            SEED,
            hv(),
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut own,
            0,
            0,
            CX + 1,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(last(&ev).2, REFUSE_D_OVERLAP);
    }

    #[test]
    fn bag_cap_holds_per_owner() {
        let dc = DeployContent::probe_fixture();
        let bc = BuildContent::probe_fixture();
        let mut pieces = Pieces::new();
        let mut deploys = Deploys::new();
        let mut ev = EventQueue::default();
        let mut p = player_at_cell(CX, CZ, &[(5, 20)]);
        let mut placed = 0usize;
        // Cells along +x within reach of a moving player.
        for k in 0..BAG_CAP as u16 + 1 {
            p.body = Body::at(
                SEED,
                hv(),
                (CX + k) as f32 * crate::build::BUILD_CELL_M + 1.5,
                CZ as f32 * crate::build::BUILD_CELL_M + 1.5,
            );
            place_deploy(
                SEED,
                hv(),
                &dc,
                &bc,
                &mut pieces,
                &mut deploys,
                &mut p,
                0,
                3,
                CX + k,
                CZ,
                0,
                LOC_PLANE,
                &mut ev,
            );
            if last(&ev).0 == crate::world::EV_DEPLOY_PLACED {
                placed += 1;
            }
        }
        assert_eq!(placed, BAG_CAP);
        assert_eq!(last(&ev).2, REFUSE_D_BAG_CAP);
        // Every one of them places ready: the cap is how many answers a
        // player may stack, and a stack that had to warm up would make the
        // cap mean something else on the first death after a build.
        assert!(
            deploys.bag_ready().iter().all(|&r| r == 0),
            "a freshly placed bag must be ready for the next death"
        );
    }

    /// The knob is spoken in minutes (ALPHA §1 / DECISIONS §open, "bag
    /// cooldown · cap": 5 min) and the sim only owns ticks, so the literal
    /// the registry gate pins is checked against the arithmetic it stands
    /// for. Change `TICK_HZ` and this fires — which is the point: a
    /// cooldown that silently became eight minutes because the tick rate
    /// moved is a knob nobody spoke.
    #[test]
    fn the_cooldown_is_five_minutes_of_ticks() {
        assert_eq!(
            BAG_COOLDOWN_TICKS,
            5 * 60 * crate::limits::TICK_HZ as u64,
            "BAG_COOLDOWN_TICKS is no longer the 5 minutes DECISIONS.md §open declares"
        );
    }

    /// Three bags of one owner, and the scan takes the nearest to the body
    /// — then refuses to take it twice. Bag geometry, at the level the
    /// world cannot see: `World::respawn` only ever asks this one question
    /// and this is where the answer is made.
    #[test]
    fn the_nearest_ready_bag_answers_and_is_then_spent() {
        let dc = DeployContent::probe_fixture();
        let bc = BuildContent::probe_fixture();
        let mut pieces = Pieces::new();
        let mut deploys = Deploys::new();
        let mut ev = EventQueue::default();
        let mut p = player_at_cell(CX, CZ, &[(5, 20)]);
        // Three bags in a row along +x, placed near-then-far so "nearest"
        // cannot be satisfied by "first in the store".
        for k in [0u16, 2, 4] {
            p.body = Body::at(
                SEED,
                hv(),
                (CX + k) as f32 * crate::build::BUILD_CELL_M + 1.5,
                CZ as f32 * crate::build::BUILD_CELL_M + 1.5,
            );
            place_deploy(
                SEED,
                hv(),
                &dc,
                &bc,
                &mut pieces,
                &mut deploys,
                &mut p,
                0,
                3,
                CX + k,
                CZ,
                0,
                LOC_PLANE,
                &mut ev,
            );
            assert_eq!(last(&ev).0, crate::world::EV_DEPLOY_PLACED);
        }
        assert_eq!(deploys.len(), 3);
        let center = |cx: u16| cell_center(cx, CZ);

        // Die beside the far bag: the far bag answers, not the first one.
        let (fx, fz) = center(CX + 4);
        assert_eq!(
            deploys.claim_bag(&dc, 7, fx, fz, 100),
            Some((fx, fz)),
            "the scan did not take the bag nearest the body"
        );
        // …and it is spent for exactly the cooldown, so the same death
        // point now walks to the next-nearest instead.
        let (mx, mz) = center(CX + 2);
        assert_eq!(
            deploys.claim_bag(&dc, 7, fx, fz, 100),
            Some((mx, mz)),
            "a bag answered twice inside its own cooldown"
        );
        // One tick before it wakes it is still spent; on its own tick it
        // answers again. Off-by-one on a five-minute timer is the kind of
        // bug that only ever shows up in someone's raid.
        let ready = 100 + BAG_COOLDOWN_TICKS;
        assert_eq!(
            deploys.claim_bag(&dc, 7, fx, fz, ready - 1),
            Some(center(CX)),
            "the far bag woke early"
        );
        assert_eq!(
            deploys.claim_bag(&dc, 7, fx, fz, ready),
            Some((fx, fz)),
            "the far bag did not wake on the tick its cooldown named"
        );
    }

    /// Someone else's bag is not an answer, and neither is nothing. Both
    /// verdicts are `None`, which is what puts the body back on the ring.
    #[test]
    fn a_foreign_bag_is_not_your_bag() {
        let dc = DeployContent::probe_fixture();
        let bc = BuildContent::probe_fixture();
        let mut pieces = Pieces::new();
        let mut deploys = Deploys::new();
        let mut ev = EventQueue::default();
        let (x, z) = cell_center(CX, CZ);

        // Nothing placed at all.
        assert_eq!(deploys.claim_bag(&dc, 7, x, z, 0), None);

        let mut p = player_at_cell(CX, CZ, &[(5, 20)]);
        place_deploy(
            SEED,
            hv(),
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut p,
            0,
            3,
            CX,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(last(&ev).0, crate::world::EV_DEPLOY_PLACED);
        assert_eq!(
            deploys.claim_bag(&dc, 8, x, z, 0),
            None,
            "a stranger woke on someone else's bag — the owner check is the \
             whole of base ownership here"
        );
        // The refused scan must not have spent the owner's bag either.
        assert_eq!(deploys.claim_bag(&dc, 7, x, z, 0), Some((x, z)));
    }

    /// A workbench is not a bed. Every other archetype is invisible to the
    /// scan, so a base full of boxes still wakes you on the ring.
    #[test]
    fn only_a_bag_answers_a_death() {
        let dc = DeployContent::probe_fixture();
        let bc = BuildContent::probe_fixture();
        let mut pieces = Pieces::new();
        let mut deploys = Deploys::new();
        let mut ev = EventQueue::default();
        // Row 1 is the workbench (PLACE_ANY, so bare ground holds it).
        let mut p = player_at_cell(CX, CZ, &[(3, 5)]);
        place_deploy(
            SEED,
            hv(),
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut p,
            0,
            1,
            CX,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(last(&ev).0, crate::world::EV_DEPLOY_PLACED);
        let (x, z) = cell_center(CX, CZ);
        assert_eq!(
            deploys.claim_bag(&dc, 7, x, z, 0),
            None,
            "a workbench answered a death"
        );
    }

    #[test]
    fn feed_fills_stock_and_upkeep_charges_it() {
        let dc = DeployContent::probe_fixture();
        let bc = BuildContent::probe_fixture();
        let mut pieces = Pieces::new();
        let mut deploys = Deploys::new();
        let mut ev = EventQueue::default();
        let mut p = player_at_cell(CX, CZ, &[(0, 250), (1, 80), (2, 1)]);
        founded_graded(&bc, &mut pieces, &mut p, CX, CZ);
        place_deploy(
            SEED,
            hv(),
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut p,
            0,
            0,
            CX,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(last(&ev).0, crate::world::EV_DEPLOY_PLACED);

        // Feed: FEED_CHUNK of item 0 (has 245 left after the foundation's
        // 5), all 80 of item 1.
        feed(&dc, &mut deploys, &mut p, CX, CZ, 0, &mut ev);
        assert_eq!(last(&ev).0, crate::world::EV_STOCK);
        assert_eq!(deploys.hearths()[0].stock[0], FEED_CHUNK);
        assert_eq!(deploys.hearths()[0].stock[1], 80);
        // Feeding an address with no hearth refuses.
        feed(&dc, &mut deploys, &mut p, CX + 3, CZ, 0, &mut ev);
        assert_eq!(last(&ev).2, REFUSE_D_HEARTH);

        // Cross one upkeep period: the foundation (cost 5 × item 0 ⇒
        // charge ceil(5·10/2400) = 1) pays from stock; nothing decays.
        let mut pc = 0u32;
        let mut dcur = 0u32;
        let tick = UPKEEP_PERIOD_TICKS + 1;
        upkeep_sweep(
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            tick,
            &mut pc,
            &mut dcur,
            &mut tick_budget(),
            &mut ev,
        );
        assert_eq!(deploys.hearths()[0].stock[0], FEED_CHUNK - 1);
        assert_eq!(pieces.len(), 1);
        assert_eq!(
            pieces.entries()[0].hp,
            bc.pieces[GRADED_FOUNDATION].hp,
            "paid pieces keep hp"
        );
    }

    #[test]
    fn unpaid_pieces_decay_away_and_take_their_deployable() {
        let dc = DeployContent::probe_fixture();
        let bc = BuildContent::probe_fixture();
        let mut pieces = Pieces::new();
        let mut deploys = Deploys::new();
        let mut ev = EventQueue::default();
        let mut p = player_at_cell(CX, CZ, &[(0, 99), (1, 99), (3, 5)]);
        founded(&bc, &mut pieces, &mut p, CX, CZ);
        // A workbench (any-class) on the foundation.
        place_deploy(
            SEED,
            hv(),
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut p,
            0,
            1,
            CX,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(last(&ev).0, crate::world::EV_DEPLOY_PLACED);

        // No hearth anywhere: the foundation decays 5 hp (5% of 100) per
        // period. 100 hp ⇒ gone by period 20; sweep with catch-up 4 needs
        // 5 passes over distinct hours.
        let mut pc = 0u32;
        let mut dcur = 0u32;
        for hour in 1..=20u64 {
            upkeep_sweep(
                &dc,
                &bc,
                &mut pieces,
                &mut deploys,
                hour * UPKEEP_PERIOD_TICKS + 1,
                &mut pc,
                &mut dcur,
                &mut tick_budget(),
                &mut ev,
            );
            if pieces.is_empty() {
                break;
            }
        }
        assert!(pieces.is_empty(), "unpaid foundation never decayed away");
        assert!(
            deploys.is_empty(),
            "the workbench on the decayed foundation must go with it"
        );
        let codes: Vec<u8> = ev.entries().iter().map(|e| e.code).collect();
        assert!(codes.contains(&crate::world::EV_PIECE_REMOVED));
        assert!(codes.contains(&crate::world::EV_DEPLOY_REMOVED));
    }

    /// **A scaffold pays no rent and gets no shelter** (twig v0,
    /// `reference/BUILDING.md` §7b.4). The half of twig that makes it a
    /// draft rather than a cheap permanent base: the sweep never charges
    /// a twig piece upkeep, so no stock can ever protect one, and at
    /// 100 %/period it is gone in a single hour under a hearth stocked to
    /// the ceiling. Its graded neighbour on the same claim survives the
    /// same sweep, which is what makes this about the rung and not about
    /// the coverage.
    #[test]
    fn twig_rots_under_a_full_hearth_and_costs_it_nothing() {
        let dc = DeployContent::probe_fixture();
        let bc = BuildContent::probe_fixture();
        let mut pieces = Pieces::new();
        let mut deploys = Deploys::new();
        let mut ev = EventQueue::default();
        let mut p = player_at_cell(CX, CZ, &[(0, 250), (1, 250), (2, 2)]);

        // A committed foundation under the hearth, and a twig wall on it.
        founded_graded(&bc, &mut pieces, &mut p, CX, CZ);
        crate::build::place(
            SEED,
            hv(),
            &bc,
            &deploys,
            &mut pieces,
            &mut p,
            0,
            1,
            CX,
            CZ,
            0,
            LOC_EDGE_XLO,
            &mut ev,
        );
        assert_eq!(
            bc.pieces[pieces.find(CX, CZ, 0, LOC_EDGE_XLO).unwrap().row as usize].material,
            crate::build::MAT_TWIG
        );
        place_deploy(
            SEED,
            hv(),
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut p,
            0,
            0,
            CX,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(deploys.hearths().len(), 1);
        // Stocked to the ceiling in both materials: there is nothing this
        // hearth could be short of.
        deploys.hearths_mut()[0].stock = [STOCK_MAX; HEARTH_STOCK_ROWS];

        sweep_once(&dc, &bc, &mut pieces, &mut deploys, UPKEEP_PERIOD_TICKS + 1);

        assert!(
            pieces.find(CX, CZ, 0, LOC_EDGE_XLO).is_none(),
            "the twig wall survived a period — a scaffold is not a base"
        );
        assert_eq!(
            pieces.find(CX, CZ, 0, LOC_PLANE).unwrap().hp,
            bc.pieces[GRADED_FOUNDATION].hp,
            "and the graded piece on the same claim was paid for as ever"
        );
        // The one that says twig was never CHARGED, rather than charged
        // and then rotted anyway: exactly the graded foundation's row came
        // out of stock, and the twig wall's cost item is the same item 0,
        // so a charge for it would show here.
        let spent = STOCK_MAX - deploys.hearths()[0].stock[0];
        assert_eq!(
            spent,
            charge_of(
                bc.pieces[GRADED_FOUNDATION].costs[0].1,
                dc.upkeep_pct_per_day
            ),
            "the hearth paid for the foundation and nothing else"
        );
    }

    #[test]
    fn covered_deployables_are_free_uncovered_ones_decay() {
        let dc = DeployContent::probe_fixture();
        let bc = BuildContent::probe_fixture();
        let mut pieces = Pieces::new();
        let mut deploys = Deploys::new();
        let mut ev = EventQueue::default();
        let mut p = player_at_cell(CX, CZ, &[(0, 250), (1, 99), (2, 1), (5, 5)]);
        founded_graded(&bc, &mut pieces, &mut p, CX, CZ);
        place_deploy(
            SEED,
            hv(),
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut p,
            0,
            0,
            CX,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
        );
        feed(&dc, &mut deploys, &mut p, CX, CZ, 0, &mut ev);
        // A bag inside the radius and one far outside it.
        p.body = Body::at(
            SEED,
            hv(),
            (CX + 2) as f32 * crate::build::BUILD_CELL_M + 1.5,
            CZ as f32 * crate::build::BUILD_CELL_M + 1.5,
        );
        place_deploy(
            SEED,
            hv(),
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut p,
            0,
            3,
            CX + 2,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(last(&ev).0, crate::world::EV_DEPLOY_PLACED);
        let far = CX + 40;
        p.body = Body::at(
            SEED,
            hv(),
            far as f32 * crate::build::BUILD_CELL_M + 1.5,
            CZ as f32 * crate::build::BUILD_CELL_M + 1.5,
        );
        place_deploy(
            SEED,
            hv(),
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut p,
            0,
            3,
            far,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(last(&ev).0, crate::world::EV_DEPLOY_PLACED);

        let near_hp = deploys.find(CX + 2, CZ, 0, LOC_PLANE).unwrap().hp;
        let mut pc = 0u32;
        let mut dcur = 0u32;
        upkeep_sweep(
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            UPKEEP_PERIOD_TICKS + 1,
            &mut pc,
            &mut dcur,
            &mut tick_budget(),
            &mut ev,
        );
        assert_eq!(
            deploys.find(CX + 2, CZ, 0, LOC_PLANE).unwrap().hp,
            near_hp,
            "covered deployables pay nothing and lose nothing"
        );
        let far_rec = deploys.find(far, CZ, 0, LOC_PLANE).unwrap();
        assert!(
            far_rec.hp < dc.defs[3].hp,
            "uncovered deployables decay ({} vs {})",
            far_rec.hp,
            dc.defs[3].hp
        );
    }

    /// A corridor of foundations `n` cells long from `(100, 100)`, a
    /// hearth on the first cell with `stock` of material row 0 — the
    /// long-base fixture the shape tests below share. Built by writing
    /// the stores directly, for `Pieces::insert_for_test`'s stated
    /// reason: the verbs ask the claim, and these tests are about it.
    ///
    /// **Row 5, the STONE foundation, not row 0.** These tests ask what a
    /// hearth protects, and since twig v0 the answer for a twig piece is
    /// "nothing, ever" — a corridor of scaffold would rot end to end
    /// whatever the claim said, and every one of them would pass for the
    /// wrong reason or fail for one. Row 5 costs the same item 0 the
    /// hearth is stocked with here.
    fn corridor(bc: &BuildContent, n: u16, stock: u32) -> (Pieces, Deploys) {
        let mut pieces = Pieces::new();
        let mut deploys = Deploys::new();
        for cx in 100..100 + n {
            pieces.insert_for_test(cx, 100, 0, LOC_PLANE, GRADED_FOUNDATION as u8, bc);
        }
        deploys.push_hearth_for_test(100, 100, 0, 7);
        deploys.hearths_mut()[0].stock[0] = stock;
        (pieces, deploys)
    }

    fn sweep_once(
        dc: &DeployContent,
        bc: &BuildContent,
        pieces: &mut Pieces,
        deploys: &mut Deploys,
        tick: u64,
    ) {
        let mut ev = EventQueue::default();
        let (mut pc, mut dcur) = (0u32, 0u32);
        upkeep_sweep(
            dc,
            bc,
            pieces,
            deploys,
            tick,
            &mut pc,
            &mut dcur,
            &mut tick_budget(),
            &mut ev,
        );
    }

    /// The upkeep sweep asks the base's own shape now, not a circle
    /// (`NOW.md` §0aa item 1). A corridor twenty cells long puts its far
    /// end 57 m from the hearth: under `HEARTH_RADIUS_M` that end rotted
    /// with a stocked hearth standing at the near one; under the cached
    /// claim volume every cell is covered, because the structure reaches
    /// it — which is the sentence privilege v1 already bought the build
    /// verbs.
    #[test]
    fn upkeep_covers_the_far_end_of_a_long_base() {
        let dc = DeployContent::probe_fixture();
        let bc = BuildContent::probe_fixture();
        let (mut pieces, mut deploys) = corridor(&bc, 20, 1_000);
        // Pin the fixture's meaning against constant drift: the far cell
        // must genuinely outrun the old circle.
        const {
            assert!(
                19.0 * crate::build::BUILD_CELL_M > HEARTH_RADIUS_M,
                "the fixture no longer outruns the circle it is about"
            )
        };
        sweep_once(&dc, &bc, &mut pieces, &mut deploys, UPKEEP_PERIOD_TICKS + 1);
        for p in pieces.entries() {
            assert_eq!(
                p.hp, bc.pieces[GRADED_FOUNDATION].hp,
                "cell {} went unpaid on a base its own hearth reaches",
                p.cx
            );
        }
        assert_eq!(
            deploys.hearths()[0].stock[0],
            1_000 - 20,
            "and every cell paid its row rather than being covered free"
        );
    }

    /// The inverse gate: a detached piece — and a detached deployable —
    /// inside the old 24 m circle but outside the base's 16 m cushion is
    /// NOT covered any more. The circle over-claimed open ground on every
    /// side a base does not extend to; the shape does not.
    #[test]
    fn a_detached_neighbor_inside_the_old_circle_is_not_covered() {
        let dc = DeployContent::probe_fixture();
        let bc = BuildContent::probe_fixture();
        let (mut pieces, mut deploys) = corridor(&bc, 1, 1_000);
        // Seven cells is 21 m: inside the circle, outside the cushion —
        // pinned, so a constant move cannot quietly hollow this test.
        let d = 7.0 * crate::build::BUILD_CELL_M;
        assert!(
            d <= HEARTH_RADIUS_M && d > crate::claim::PRIV_CUSHION_M,
            "the fixture distance no longer separates circle from cushion"
        );
        pieces.insert_for_test(107, 100, 0, LOC_PLANE, GRADED_FOUNDATION as u8, &bc);
        // A workbench the same distance the other way, on open ground.
        assert!(deploys.insert(
            DeployRec {
                cx: 100,
                cz: 107,
                level: 0,
                loc: LOC_PLANE,
                row: 1,
                owner: 7,
                hp: dc.defs[1].hp,
                uh: 0,
                open: false,
                has_lock: false,
                locked: false,
                dmg: 0,
            },
            0,
        ));
        sweep_once(&dc, &bc, &mut pieces, &mut deploys, UPKEEP_PERIOD_TICKS + 1);
        assert_eq!(
            pieces.find(100, 100, 0, LOC_PLANE).unwrap().hp,
            bc.pieces[GRADED_FOUNDATION].hp,
            "the piece the base is made of is paid for"
        );
        assert!(
            pieces.find(107, 100, 0, LOC_PLANE).unwrap().hp < bc.pieces[GRADED_FOUNDATION].hp,
            "a detached piece the structure does not reach decays, however \
             close the hearth stands"
        );
        assert!(
            deploys.find(100, 107, 0, LOC_PLANE).unwrap().hp < dc.defs[1].hp,
            "and so does a detached deployable"
        );
        assert_eq!(
            deploys.hearths()[0].stock[0],
            1_000 - 1,
            "only the piece inside the shape was charged"
        );
    }

    /// The cache-invalidation mutant-killer `NOW.md` §0aa item 2 asks
    /// for: place, sweep, demolish the corridor, sweep — the second sweep
    /// must see the shrunk shape. Delete any link in the invalidation
    /// chain (the gen bump in `Pieces::remove_at`, the stamp compare, the
    /// refresh call at the sweep's top) and the far piece stays covered
    /// by structure that is no longer there, and this test reddens.
    #[test]
    fn the_sweep_sees_a_demolished_base_shrink() {
        let dc = DeployContent::probe_fixture();
        let bc = BuildContent::probe_fixture();
        let (mut pieces, mut deploys) = corridor(&bc, 10, 1_000);
        // The far cell is 27 m out: outside the circle AND outside the
        // lone foundation's cushion, so after the demolition below only
        // the shape argument can protect or rot it.
        const { assert!(9.0 * crate::build::BUILD_CELL_M > HEARTH_RADIUS_M) };
        sweep_once(&dc, &bc, &mut pieces, &mut deploys, UPKEEP_PERIOD_TICKS + 1);
        assert_eq!(
            pieces.find(109, 100, 0, LOC_PLANE).unwrap().hp,
            bc.pieces[GRADED_FOUNDATION].hp,
            "before the demolition the far end is covered"
        );
        assert_eq!(deploys.hearths()[0].stock[0], 1_000 - 10);
        // Demolish the corridor between — the same `remove_at` every
        // removal path funnels through, so the gen bump under test is the
        // one the live verbs exercise.
        for cx in 101..109u16 {
            let i = pieces.find_index(cx, 100, 0, LOC_PLANE).unwrap();
            let shape = bc.pieces[pieces.entries()[i].row as usize].shape;
            pieces.remove_at(i, shape);
        }
        sweep_once(
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            2 * UPKEEP_PERIOD_TICKS + 1,
        );
        assert_eq!(
            pieces.find(100, 100, 0, LOC_PLANE).unwrap().hp,
            bc.pieces[GRADED_FOUNDATION].hp,
            "the cell the hearth stands on is still covered"
        );
        assert!(
            pieces.find(109, 100, 0, LOC_PLANE).unwrap().hp < bc.pieces[GRADED_FOUNDATION].hp,
            "the second sweep must see the shrunk shape — a far piece the \
             demolition detached has to rot, and if it did not, the claim \
             cache was not invalidated"
        );
    }

    /// A −x strafe at yaw 0 (forward +Z, right +X — the collide.rs
    /// test convention).
    fn walk_minus_x() -> crate::input::InputFrame {
        crate::input::InputFrame {
            seq: 1,
            move_x: -127,
            ..crate::input::InputFrame::default()
        }
    }

    /// Foundation + doorway (build row 3) + door (deploy row 2) on the
    /// low-x edge of (CX, CZ); returns the acting player.
    fn doored(
        bc: &BuildContent,
        dc: &DeployContent,
        pieces: &mut Pieces,
        deploys: &mut Deploys,
        ev: &mut EventQueue,
    ) -> Player {
        let mut p = player_at_cell(CX, CZ, &[(0, 99), (1, 99), (4, 2), (7, 2)]);
        founded(bc, pieces, &mut p, CX, CZ);
        crate::build::place(
            SEED,
            hv(),
            bc,
            deploys,
            pieces,
            &mut p,
            0,
            3,
            CX,
            CZ,
            0,
            LOC_EDGE_XLO,
            ev,
        );
        assert_eq!(last(ev).0, crate::world::EV_PIECE_PLACED, "doorway lands");
        place_deploy(
            SEED,
            hv(),
            dc,
            bc,
            pieces,
            deploys,
            &mut p,
            0,
            2,
            CX,
            CZ,
            0,
            LOC_EDGE_XLO,
            ev,
        );
        assert_eq!(last(ev).0, crate::world::EV_DEPLOY_PLACED, "door lands");
        p
    }

    /// Walk a fresh body −x through the doorway; the x it pins at.
    fn walk_x_after(pieces: &Pieces) -> f32 {
        let mut b = crate::movement::Body::at(
            SEED,
            hv(),
            CX as f32 * crate::build::BUILD_CELL_M + 1.5,
            CZ as f32 * crate::build::BUILD_CELL_M + 1.5,
        );
        let f = walk_minus_x();
        // The door is what this fixture is about; a pine standing where it
        // walks is not (occupy::Barren).
        let mut occ = crate::occupy::Scratch::barren();
        for _ in 0..120 {
            crate::movement::step(SEED, hv(), pieces.cols(), &mut occ.occupants(), &mut b, &f);
        }
        b.qx as f32 * crate::movement::POS_XZ_Q
    }

    /// The door and the doorway it hangs in have the *same* address, and
    /// this is the gate on the bit that tells them apart.
    ///
    /// `place_deploy`'s `PLACE_DOORWAY` arm requires the piece at the
    /// identical `(cx, cz, level, loc)`, so the two stores overlap here by
    /// construction rather than by accident. A repair that read the wrong
    /// store would find a real record at a real address, price it against
    /// a real row, take real materials and mend the wrong thing — every
    /// step of it looking correct. Nothing else in the payload could catch
    /// that, which is why the bit is on the wire and not inferred.
    #[test]
    fn the_deploy_bit_mends_the_door_and_leaves_the_doorway_alone() {
        let dc = DeployContent::probe_fixture();
        let bc = BuildContent::probe_fixture();
        let mut pieces = Pieces::new();
        let mut deploys = Deploys::new();
        let mut ev = EventQueue::default();
        let mut p = doored(&bc, &dc, &mut pieces, &mut deploys, &mut ev);

        let di = deploys
            .find_index(CX, CZ, 0, LOC_EDGE_XLO)
            .expect("the door");
        let pi = pieces
            .find_index(CX, CZ, 0, LOC_EDGE_XLO)
            .expect("the doorway");
        deploys.set_hp(di, 30);
        pieces.set_hp(pi, 50);
        let wood = crate::craft::inv_count(&p.inv, 0);
        let cloth = crate::craft::inv_count(&p.inv, 1);

        crate::build::repair(
            &bc,
            &dc,
            &mut deploys,
            &mut pieces,
            &mut p,
            true,
            CX,
            CZ,
            0,
            LOC_EDGE_XLO,
            &mut ev,
        );
        assert_eq!(
            deploys.entries()[di].hp,
            60,
            "the DOOR is what the set bit names, and it stands at its \
             baked 60"
        );
        assert_eq!(
            pieces.entries()[pi].hp,
            50,
            "and the doorway at the same address is untouched — mending it \
             here would be the whole bug this bit exists to prevent"
        );
        // 40 units over 60 hp, 30 missing, at 100%: 20. The second row is
        // 8 over the same, which is 4 — a deployable priced from its
        // recipe, not from the one crafted item placing it took.
        assert_eq!(crate::craft::inv_count(&p.inv, 0), wood - 20);
        assert_eq!(crate::craft::inv_count(&p.inv, 1), cloth - 4);

        crate::build::repair(
            &bc,
            &dc,
            &mut deploys,
            &mut pieces,
            &mut p,
            false,
            CX,
            CZ,
            0,
            LOC_EDGE_XLO,
            &mut ev,
        );
        assert_eq!(
            pieces.entries()[pi].hp,
            100,
            "and the clear bit reaches the doorway, at the same address"
        );
        // The piece table's own row: 3 units over 100 hp, 50 missing = 2,
        // and only the one item, so the door's second row is not charged.
        assert_eq!(crate::craft::inv_count(&p.inv, 0), wood - 22);
        assert_eq!(crate::craft::inv_count(&p.inv, 1), cloth - 4);
    }

    /// The bit selects a store; it does not fall back to the other one.
    ///
    /// A plain wall has no deployable at its address. Asking to repair a
    /// deployable there must refuse rather than quietly mend the wall,
    /// because a client that had the wrong idea about what it was aiming
    /// at should be told, not served.
    #[test]
    fn a_deploy_repair_at_a_piece_only_address_refuses() {
        let dc = DeployContent::probe_fixture();
        let bc = BuildContent::probe_fixture();
        let mut pieces = Pieces::new();
        let mut deploys = Deploys::new();
        let mut ev = EventQueue::default();
        let mut p = player_at_cell(CX, CZ, &[(0, 99), (1, 99)]);
        founded(&bc, &mut pieces, &mut p, CX, CZ);
        crate::build::place(
            SEED,
            hv(),
            &bc,
            &deploys,
            &mut pieces,
            &mut p,
            0,
            1,
            CX,
            CZ,
            0,
            LOC_EDGE_ZLO,
            &mut ev,
        );
        assert_eq!(last(&ev).0, crate::world::EV_PIECE_PLACED, "the wall lands");
        let pi = pieces
            .find_index(CX, CZ, 0, LOC_EDGE_ZLO)
            .expect("the wall");
        pieces.set_hp(pi, 40);
        let wood = crate::craft::inv_count(&p.inv, 0);

        crate::build::repair(
            &bc,
            &dc,
            &mut deploys,
            &mut pieces,
            &mut p,
            true,
            CX,
            CZ,
            0,
            LOC_EDGE_ZLO,
            &mut ev,
        );
        assert_eq!(
            last(&ev),
            (crate::world::EV_BUILD_REFUSED, p.id, REFUSE_B_SPOT, 0),
            "no deployable stands there, and the piece at the same address \
             is not a substitute for one"
        );
        assert_eq!(pieces.entries()[pi].hp, 40, "the wall is untouched");
        assert_eq!(crate::craft::inv_count(&p.inv, 0), wood, "and unpaid for");
    }

    /// A row that quotes no price is refused, not mended free.
    ///
    /// `DeployDef::n_costs` is 0 whenever the bake found no recipe for the
    /// deployable's item. Both cost loops in `repair` iterate `n_costs`
    /// rows, so without this refusal the check loop passes vacuously, the
    /// take loop takes nothing, and the hp assignment still runs — a full
    /// heal for free, on a path where every individual step looks right.
    #[test]
    fn an_unpriced_deployable_refuses_rather_than_mending_free() {
        let mut dc = DeployContent::probe_fixture();
        let bc = BuildContent::probe_fixture();
        let mut pieces = Pieces::new();
        let mut deploys = Deploys::new();
        let mut ev = EventQueue::default();
        let mut p = doored(&bc, &dc, &mut pieces, &mut deploys, &mut ev);
        let di = deploys
            .find_index(CX, CZ, 0, LOC_EDGE_XLO)
            .expect("the door");
        deploys.set_hp(di, 30);
        let wood = crate::craft::inv_count(&p.inv, 0);

        // Content with no recipe behind this door: the row still has hp
        // and an item, it simply has no price.
        dc.defs[2].n_costs = 0;
        crate::build::repair(
            &bc,
            &dc,
            &mut deploys,
            &mut pieces,
            &mut p,
            true,
            CX,
            CZ,
            0,
            LOC_EDGE_XLO,
            &mut ev,
        );
        assert_eq!(
            last(&ev),
            (crate::world::EV_BUILD_REFUSED, p.id, REFUSE_B_UNPRICED, 0),
            "an unpriced row is refused by name"
        );
        assert_eq!(
            deploys.entries()[di].hp,
            30,
            "and the door stays damaged — a free mend is the one outcome \
             this refusal exists to make impossible"
        );
        assert_eq!(crate::craft::inv_count(&p.inv, 0), wood);
    }

    #[test]
    fn door_places_closed_toggles_open_and_reseals() {
        let dc = DeployContent::probe_fixture();
        let bc = BuildContent::probe_fixture();
        let mut pieces = Pieces::new();
        let mut deploys = Deploys::new();
        let mut ev = EventQueue::default();
        let mut p = doored(&bc, &dc, &mut pieces, &mut deploys, &mut ev);
        let wall_x = CX as f32 * crate::build::BUILD_CELL_M;

        // Placed closed: sim state, the shut bit, and the walk agree.
        assert!(!deploys.find(CX, CZ, 0, LOC_EDGE_XLO).unwrap().open);
        assert!(
            !deploys.find(CX, CZ, 0, LOC_EDGE_XLO).unwrap().has_lock
                && !deploys.find(CX, CZ, 0, LOC_EDGE_XLO).unwrap().locked,
            "a door places BARE (lock v1) — the security is what costs"
        );
        assert_eq!(pieces.cols().get(CX, CZ).shut_xlo, 1);
        assert!(
            walk_x_after(&pieces) >= wall_x,
            "a closed door must block the doorway opening"
        );

        // Use opens: EV_DOOR announces, the same walk passes.
        use_door(
            &dc,
            &mut pieces,
            &mut deploys,
            &mut p,
            CX,
            CZ,
            0,
            LOC_EDGE_XLO,
            &mut ev,
        );
        assert_eq!(
            last(&ev),
            (
                crate::world::EV_DOOR,
                crate::gather::cell_key(CX, CZ),
                // open, no lock — the announcement is the whole door,
                // absolute (lock v1: has_lock << 2 | locked << 1 | open).
                (LOC_EDGE_XLO as u32) << 8 | 1,
                7
            )
        );
        assert!(deploys.find(CX, CZ, 0, LOC_EDGE_XLO).unwrap().open);
        assert_eq!(pieces.cols().get(CX, CZ).shut_xlo, 0);
        assert!(
            walk_x_after(&pieces) < wall_x - 0.5,
            "an open door must pass like an empty doorway"
        );

        // Use again reseals.
        use_door(
            &dc,
            &mut pieces,
            &mut deploys,
            &mut p,
            CX,
            CZ,
            0,
            LOC_EDGE_XLO,
            &mut ev,
        );
        assert_eq!(last(&ev).2 & 1, 0, "EV_DOOR carries open = 0");
        assert_eq!(pieces.cols().get(CX, CZ).shut_xlo, 1);
        assert!(
            walk_x_after(&pieces) >= wall_x,
            "reclosed door blocks again"
        );
    }

    /// Bolt the probe fixture's code lock (row 4) onto the door `doored`
    /// hung, and arm it with `code`. Returns nothing: every assertion the
    /// callers make is about the store, which is where the truth is.
    fn locked_door(
        dc: &DeployContent,
        deploys: &mut Deploys,
        p: &mut Player,
        code: u16,
        ev: &mut EventQueue,
    ) {
        let bc = BuildContent::probe_fixture();
        let mut pieces_unused = Pieces::new();
        let _ = &mut pieces_unused;
        let _ = &bc;
        lock_op(
            dc,
            &GatherContent::probe_fixture(),
            deploys,
            p,
            CX,
            CZ,
            0,
            LOC_EDGE_XLO,
            crate::deploy::ACCESS_OP_SET_CODE,
            code,
            0,
            ev,
            &mut [ItemStack::default(); INV_SLOTS],
        );
    }

    /// The whole lock v1 access model on one door: bare, then bolted, then
    /// armed, then shared two ways.
    #[test]
    fn a_bare_door_is_anyones_and_a_lock_is_what_claims_it() {
        let dc = DeployContent::probe_fixture();
        let bc = BuildContent::probe_fixture();
        let gc = GatherContent::probe_fixture();
        let mut pieces = Pieces::new();
        let mut deploys = Deploys::new();
        let mut ev = EventQueue::default();
        let mut p = doored(&bc, &dc, &mut pieces, &mut deploys, &mut ev);
        let mut stranger = player_at_cell(CX, CZ, &[]);
        stranger.id = 9;

        // Bare: the stranger works it, which is the reference's rule and
        // the reason a lock costs anything at all.
        use_door(
            &dc,
            &mut pieces,
            &mut deploys,
            &mut stranger,
            CX,
            CZ,
            0,
            LOC_EDGE_XLO,
            &mut ev,
        );
        assert_eq!(
            last(&ev).0,
            crate::world::EV_DOOR,
            "a bare door is anyone's"
        );
        assert!(deploys.find(CX, CZ, 0, LOC_EDGE_XLO).unwrap().open);

        // Bolt one on. Placing the lock mints no deploy record — it is a
        // record *about* one — and the door announces its new bit.
        let before = deploys.len();
        place_deploy(
            SEED,
            hv(),
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut p,
            0,
            5,
            CX,
            CZ,
            0,
            LOC_EDGE_XLO,
            &mut ev,
        );
        assert_eq!(deploys.len(), before, "a lock is not a deployable record");
        assert_eq!(deploys.locks().len(), 1, "it is a lock record");
        let d = deploys.find(CX, CZ, 0, LOC_EDGE_XLO).unwrap();
        assert!(d.has_lock && !d.locked, "bolted on, not yet armed");
        assert_eq!(
            last(&ev).2 & 7,
            4 | 1,
            "EV_DOOR carries has_lock << 2 over the open leaf"
        );

        // Unarmed, it still lets everyone through: arming is set_code.
        assert!(deploys.lock_passes(CX, CZ, 0, LOC_EDGE_XLO, stranger.id));

        // Arm it. Now the stranger bounces — and knocks.
        locked_door(&dc, &mut deploys, &mut p, 1234, &mut ev);
        assert!(deploys.find(CX, CZ, 0, LOC_EDGE_XLO).unwrap().locked);
        let open_before = deploys.find(CX, CZ, 0, LOC_EDGE_XLO).unwrap().open;
        use_door(
            &dc,
            &mut pieces,
            &mut deploys,
            &mut stranger,
            CX,
            CZ,
            0,
            LOC_EDGE_XLO,
            &mut ev,
        );
        assert_eq!(
            (last(&ev).0, last(&ev).2),
            (crate::world::EV_DEPLOY_REFUSED, REFUSE_D_OWNER),
            "a locked door refuses a hand it does not know"
        );
        assert_eq!(
            ev.entries()[ev.len() - 2].code,
            crate::world::EV_KNOCK,
            "and the refusal KNOCKS — the one channel a locked-out player has"
        );
        assert_eq!(
            deploys.find(CX, CZ, 0, LOC_EDGE_XLO).unwrap().open,
            open_before,
            "the leaf did not move"
        );

        // The code is the mechanic: entering it authorizes, it does not
        // open. The door does not move on this press.
        let open_at_entry = deploys.find(CX, CZ, 0, LOC_EDGE_XLO).unwrap().open;
        lock_op(
            &dc,
            &gc,
            &mut deploys,
            &mut stranger,
            CX,
            CZ,
            0,
            LOC_EDGE_XLO,
            crate::deploy::ACCESS_OP_ENTER,
            1234,
            0,
            &mut ev,
            &mut [ItemStack::default(); INV_SLOTS],
        );
        assert_eq!(
            (last(&ev).0, last(&ev).2 & 0xff, last(&ev).3),
            (
                crate::world::EV_AUTH,
                crate::lock::GRANT_FULL as u32,
                stranger.id
            ),
            "a correct code announces the grant, to its sender only"
        );
        assert_eq!(
            deploys.find(CX, CZ, 0, LOC_EDGE_XLO).unwrap().open,
            open_at_entry,
            "entering a code is not opening a door"
        );
        use_door(
            &dc,
            &mut pieces,
            &mut deploys,
            &mut stranger,
            CX,
            CZ,
            0,
            LOC_EDGE_XLO,
            &mut ev,
        );
        assert_eq!(
            last(&ev).0,
            crate::world::EV_DOOR,
            "and thereafter the door simply works"
        );
    }

    /// A wrong code costs hp, escalating, and never the last point of it.
    #[test]
    fn a_wrong_code_shocks_and_the_eighth_shuts_the_keypad() {
        let dc = DeployContent::probe_fixture();
        let bc = BuildContent::probe_fixture();
        let gc = GatherContent::probe_fixture();
        let mut pieces = Pieces::new();
        let mut deploys = Deploys::new();
        let mut ev = EventQueue::default();
        let mut p = doored(&bc, &dc, &mut pieces, &mut deploys, &mut ev);
        place_deploy(
            SEED,
            hv(),
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut p,
            0,
            5,
            CX,
            CZ,
            0,
            LOC_EDGE_XLO,
            &mut ev,
        );
        locked_door(&dc, &mut deploys, &mut p, 1234, &mut ev);

        let mut raider = player_at_cell(CX, CZ, &[]);
        raider.id = 9;
        raider.hp = 100;
        raider.hp_max = 100;
        let wrong = |deploys: &mut Deploys, r: &mut Player, ev: &mut EventQueue| {
            lock_op(
                &dc,
                &gc,
                deploys,
                r,
                CX,
                CZ,
                0,
                LOC_EDGE_XLO,
                crate::deploy::ACCESS_OP_ENTER,
                4321,
                0,
                ev,
                &mut [ItemStack::default(); INV_SLOTS],
            );
        };
        wrong(&mut deploys, &mut raider, &mut ev);
        assert_eq!(raider.hp, 95, "the first miss is one step");
        assert_eq!(last(&ev).2, REFUSE_D_CODE);
        wrong(&mut deploys, &mut raider, &mut ev);
        assert_eq!(raider.hp, 85, "the second is two");
        for _ in 0..6 {
            wrong(&mut deploys, &mut raider, &mut ev);
        }
        assert_eq!(
            last(&ev).2,
            REFUSE_D_LOCKOUT,
            "the eighth miss shuts the keypad"
        );
        // ...and the *correct* code is refused while it is shut, which is
        // the point of a lockout.
        lock_op(
            &dc,
            &gc,
            &mut deploys,
            &mut raider,
            CX,
            CZ,
            0,
            LOC_EDGE_XLO,
            crate::deploy::ACCESS_OP_ENTER,
            1234,
            0,
            &mut ev,
            &mut [ItemStack::default(); INV_SLOTS],
        );
        assert_eq!(last(&ev).2, REFUSE_D_LOCKOUT);
        assert!(!deploys.lock_passes(CX, CZ, 0, LOC_EDGE_XLO, raider.id));

        // The floor: a body cannot be finished by a keypad.
        raider.hp = 1;
        wrong(&mut deploys, &mut raider, &mut ev);
        assert_eq!(raider.hp, 1, "a lock may maim, never kill");
    }

    /// The lock verb's shape: full-rights ops bounce for a stranger, the
    /// address must hold a door and a lock, and reach is the door's own.
    #[test]
    fn lock_ops_bounce_on_rights_reach_and_a_missing_lock() {
        let dc = DeployContent::probe_fixture();
        let bc = BuildContent::probe_fixture();
        let gc = GatherContent::probe_fixture();
        let mut pieces = Pieces::new();
        let mut deploys = Deploys::new();
        let mut ev = EventQueue::default();
        let mut p = doored(&bc, &dc, &mut pieces, &mut deploys, &mut ev);

        // No lock on the door yet: every op says so, rather than saying
        // "not yours" — a client that could not tell those apart would
        // prompt for a code at a door with no keypad.
        lock_op(
            &dc,
            &gc,
            &mut deploys,
            &mut p,
            CX,
            CZ,
            0,
            LOC_EDGE_XLO,
            crate::deploy::ACCESS_OP_UNLOCK,
            0,
            0,
            &mut ev,
            &mut [ItemStack::default(); INV_SLOTS],
        );
        assert_eq!(last(&ev).2, REFUSE_D_NO_LOCK);

        place_deploy(
            SEED,
            hv(),
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut p,
            0,
            5,
            CX,
            CZ,
            0,
            LOC_EDGE_XLO,
            &mut ev,
        );
        // One lock per door.
        place_deploy(
            SEED,
            hv(),
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut p,
            0,
            5,
            CX,
            CZ,
            0,
            LOC_EDGE_XLO,
            &mut ev,
        );
        assert_eq!(last(&ev).2, REFUSE_D_HAS_LOCK);
        locked_door(&dc, &mut deploys, &mut p, 1234, &mut ev);

        // A stranger may not arm, disarm, or unbolt it.
        let mut stranger = player_at_cell(CX, CZ, &[]);
        stranger.id = 9;
        for op in [
            crate::deploy::ACCESS_OP_SET_CODE,
            crate::deploy::ACCESS_OP_SET_GUEST,
            crate::deploy::ACCESS_OP_LOCK,
            crate::deploy::ACCESS_OP_UNLOCK,
            crate::deploy::ACCESS_OP_TAKE,
        ] {
            lock_op(
                &dc,
                &gc,
                &mut deploys,
                &mut stranger,
                CX,
                CZ,
                0,
                LOC_EDGE_XLO,
                op,
                1111,
                0,
                &mut ev,
                &mut [ItemStack::default(); INV_SLOTS],
            );
            assert_eq!(last(&ev).2, REFUSE_D_OWNER, "op {op} is full-rights");
        }
        assert_eq!(deploys.locks().len(), 1, "and the lock is still on");

        // The two refusals the use verb bounces with, unchanged.
        lock_op(
            &dc,
            &gc,
            &mut deploys,
            &mut p,
            CX,
            CZ,
            0,
            LOC_PLANE,
            crate::deploy::ACCESS_OP_UNLOCK,
            0,
            0,
            &mut ev,
            &mut [ItemStack::default(); INV_SLOTS],
        );
        assert_eq!(last(&ev).2, REFUSE_D_DOOR, "no door at that address");
        let mut far = player_at_cell(CX + 7, CZ, &[]);
        lock_op(
            &dc,
            &gc,
            &mut deploys,
            &mut far,
            CX,
            CZ,
            0,
            LOC_EDGE_XLO,
            crate::deploy::ACCESS_OP_UNLOCK,
            0,
            0,
            &mut ev,
            &mut [ItemStack::default(); INV_SLOTS],
        );
        assert_eq!(last(&ev).2, REFUSE_D_REACH, "a lock has the build reach");
        assert!(
            deploys.find(CX, CZ, 0, LOC_EDGE_XLO).unwrap().locked,
            "a refused op leaves the bit where it was"
        );

        // Taking it off returns the item and makes the door anyone's.
        let held = crate::craft::inv_count(&p.inv, 7);
        lock_op(
            &dc,
            &gc,
            &mut deploys,
            &mut p,
            CX,
            CZ,
            0,
            LOC_EDGE_XLO,
            crate::deploy::ACCESS_OP_TAKE,
            0,
            0,
            &mut ev,
            &mut [ItemStack::default(); INV_SLOTS],
        );
        assert_eq!(deploys.locks().len(), 0);
        assert_eq!(
            crate::craft::inv_count(&p.inv, 7),
            held + 1,
            "the item comes back"
        );
        let d = deploys.find(CX, CZ, 0, LOC_EDGE_XLO).unwrap();
        assert!(!d.has_lock && !d.locked, "and both mirror bits cleared");
        assert!(deploys.lock_passes(CX, CZ, 0, LOC_EDGE_XLO, stranger.id));
    }

    /// Pickup v1: a deployable comes back up any time you may build
    /// there, and comes up **empty or not at all**.
    #[test]
    fn a_deployable_comes_back_up_and_a_full_one_does_not() {
        let dc = DeployContent::probe_fixture();
        let bc = BuildContent::probe_fixture();
        let gc = GatherContent::probe_fixture();
        let mut pieces = Pieces::new();
        let mut deploys = Deploys::new();
        let mut ev = EventQueue::default();
        let mut p = player_at_cell(CX, CZ, &[(0, 99), (1, 99), (2, 2), (3, 2)]);
        founded(&bc, &mut pieces, &mut p, CX, CZ);

        // A workbench (row 1, item 3) on the foundation. No hearth stands,
        // so the ground is unclaimed and anyone in reach may lift it —
        // which is `DOORS.md` §5's rule and the reason a hearth is worth
        // placing at all.
        place_deploy(
            SEED,
            hv(),
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut p,
            0,
            1,
            CX,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(deploys.len(), 1);
        let held = crate::craft::inv_count(&p.inv, 3);
        let mut stranger = player_at_cell(CX, CZ, &[]);
        stranger.id = 9;
        pick_up(
            &dc,
            &gc,
            &mut pieces,
            &mut deploys,
            &mut stranger,
            CX,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
            &mut [ItemStack::default(); INV_SLOTS],
        );
        assert_eq!(deploys.len(), 0, "unclaimed furniture is anyone's");
        assert_eq!(
            crate::craft::inv_count(&stranger.inv, 3),
            1,
            "and the item goes to the hand that lifted it"
        );
        assert_eq!(
            crate::craft::inv_count(&p.inv, 3),
            held,
            "not to the placer"
        );

        // A hearth with stock in it refuses: lifting it would take the
        // stock with it.
        place_deploy(
            SEED,
            hv(),
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut p,
            0,
            0,
            CX,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(deploys.hearths().len(), 1);
        deploys.hearths_mut()[0].stock[0] = 5;
        pick_up(
            &dc,
            &gc,
            &mut pieces,
            &mut deploys,
            &mut p,
            CX,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
            &mut [ItemStack::default(); INV_SLOTS],
        );
        assert_eq!(
            (last(&ev).0, last(&ev).2),
            (crate::world::EV_DEPLOY_REFUSED, REFUSE_D_NOT_EMPTY)
        );
        assert_eq!(deploys.hearths().len(), 1, "and it is still standing");
        deploys.hearths_mut()[0].stock[0] = 0;
        pick_up(
            &dc,
            &gc,
            &mut pieces,
            &mut deploys,
            &mut p,
            CX,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
            &mut [ItemStack::default(); INV_SLOTS],
        );
        assert_eq!(deploys.hearths().len(), 0, "emptied, it lifts");
    }

    /// A locked door cannot be lifted out of its frame by a hand the lock
    /// does not know — without this, every code lock in the game is
    /// defeated by picking up what it is bolted to.
    #[test]
    fn a_lock_guards_the_door_against_being_picked_up() {
        let dc = DeployContent::probe_fixture();
        let bc = BuildContent::probe_fixture();
        let gc = GatherContent::probe_fixture();
        let mut pieces = Pieces::new();
        let mut deploys = Deploys::new();
        let mut ev = EventQueue::default();
        let mut p = doored(&bc, &dc, &mut pieces, &mut deploys, &mut ev);
        place_deploy(
            SEED,
            hv(),
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut p,
            0,
            5,
            CX,
            CZ,
            0,
            LOC_EDGE_XLO,
            &mut ev,
        );
        locked_door(&dc, &mut deploys, &mut p, 1234, &mut ev);

        let mut stranger = player_at_cell(CX, CZ, &[]);
        stranger.id = 9;
        pick_up(
            &dc,
            &gc,
            &mut pieces,
            &mut deploys,
            &mut stranger,
            CX,
            CZ,
            0,
            LOC_EDGE_XLO,
            &mut ev,
            &mut [ItemStack::default(); INV_SLOTS],
        );
        assert_eq!(
            (last(&ev).0, last(&ev).2),
            (crate::world::EV_DEPLOY_REFUSED, REFUSE_D_OWNER),
            "the lock is what says whose door it is, and lifting is a way \
             through it"
        );
        assert_eq!(deploys.len(), 1, "the door is still hanging");

        // The hand the lock knows lifts it, and the lock comes up as a
        // second item rather than being destroyed with the frame.
        let doors = crate::craft::inv_count(&p.inv, 4);
        let locks = crate::craft::inv_count(&p.inv, 7);
        pick_up(
            &dc,
            &gc,
            &mut pieces,
            &mut deploys,
            &mut p,
            CX,
            CZ,
            0,
            LOC_EDGE_XLO,
            &mut ev,
            &mut [ItemStack::default(); INV_SLOTS],
        );
        assert_eq!(deploys.len(), 0);
        assert_eq!(deploys.locks().len(), 0, "and the lock came off with it");
        assert_eq!(crate::craft::inv_count(&p.inv, 4), doors + 1, "the door");
        assert_eq!(crate::craft::inv_count(&p.inv, 7), locks + 1, "the lock");
    }

    /// `probe_fixture` plus a storage box on row 6 (item 8). A local
    /// fixture rather than a new shipped row, deliberately: the replay
    /// golden is read off worlds built from `probe_fixture`, and a row
    /// nothing in the probe script places would still move `def_count`
    /// under it for no exercised behaviour.
    fn boxed_fixture() -> DeployContent {
        let mut d = DeployContent::probe_fixture();
        // Slot **7**, item **9** — both one past what the shared fixture
        // spends, which is the whole contract of this helper: append a row
        // the probe script never places rather than shadow one it does.
        // It sat at 6/8 until the recycler took those (recycler v0), and
        // moving it was not optional — a bespoke fixture that quietly
        // replaces a shared row makes four box tests pass while asserting
        // something about a different object.
        d.defs[7] = DeployDef {
            arch: ARCH_BOX,
            placement: PLACE_FOUNDATION,
            hp: 60,
            item: 9,
            n_costs: 0,
            costs: [(0, 0); MAX_DEPLOY_COSTS],
        };
        d.def_count = 8;
        d
    }

    /// `GatherContent::probe_fixture` with a stack ladder for the box
    /// item above — without one, `pick_up`'s give-back hands item 9 to
    /// `inv_add` at a ceiling of zero and the box silently drops.
    fn boxed_gather() -> GatherContent {
        let mut g = GatherContent::probe_fixture();
        g.stack_max[9] = 100;
        g
    }

    /// A foundation at (CX, CZ) with a box (row 7) standing on it, placed
    /// by the returned player — `doored`'s shape, one storey lower.
    fn boxed(
        bc: &BuildContent,
        dc: &DeployContent,
        pieces: &mut Pieces,
        deploys: &mut Deploys,
        ev: &mut EventQueue,
    ) -> Player {
        let mut p = player_at_cell(CX, CZ, &[(0, 99), (1, 99), (9, 2), (7, 2)]);
        founded(bc, pieces, &mut p, CX, CZ);
        place_deploy(
            SEED,
            hv(),
            dc,
            bc,
            pieces,
            deploys,
            &mut p,
            0,
            7,
            CX,
            CZ,
            0,
            LOC_PLANE,
            ev,
        );
        assert_eq!(last(ev).0, crate::world::EV_DEPLOY_PLACED, "box lands");
        p
    }

    /// Locks on boxes (`DOORS.md` §9.8): the box takes the door's lock —
    /// same store, same ops, same mirror bits — at its own plane address.
    #[test]
    fn a_code_lock_bolts_onto_a_box_and_the_same_ops_run() {
        let dc = boxed_fixture();
        let bc = BuildContent::probe_fixture();
        let gc = boxed_gather();
        let mut pieces = Pieces::new();
        let mut deploys = Deploys::new();
        let mut ev = EventQueue::default();
        let mut p = boxed(&bc, &dc, &mut pieces, &mut deploys, &mut ev);

        // Bolt the lock (row 5) onto the box's plane address. No deploy
        // record is minted — a lock is a record about one — and the box's
        // record announces its new bit on the door lane.
        let before = deploys.len();
        place_deploy(
            SEED,
            hv(),
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut p,
            0,
            5,
            CX,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(deploys.len(), before, "a lock is not a deployable record");
        assert_eq!(deploys.locks().len(), 1, "it is a lock record");
        let d = deploys.find(CX, CZ, 0, LOC_PLANE).unwrap();
        assert!(d.has_lock && !d.locked, "bolted on, not yet armed");
        assert_eq!(
            (last(&ev).0, last(&ev).2 & 7),
            (crate::world::EV_DOOR, 4),
            "EV_DOOR carries has_lock << 2, open 0 — a box has no leaf"
        );
        assert!(
            deploys.lock_passes(CX, CZ, 0, LOC_PLANE, 999),
            "unarmed, it still lets everyone through"
        );

        // Arm it. The mirror follows the store, exactly as a door's does.
        lock_op(
            &dc,
            &gc,
            &mut deploys,
            &mut p,
            CX,
            CZ,
            0,
            LOC_PLANE,
            crate::deploy::ACCESS_OP_SET_CODE,
            1234,
            0,
            &mut ev,
            &mut [ItemStack::default(); INV_SLOTS],
        );
        assert!(deploys.locks.is_locked(CX, CZ, 0, LOC_PLANE));
        assert!(
            deploys.find(CX, CZ, 0, LOC_PLANE).unwrap().locked,
            "the locked bit mirrors via is_locked"
        );
        let mut stranger = player_at_cell(CX, CZ, &[]);
        stranger.id = 9;
        assert!(!deploys.lock_passes(CX, CZ, 0, LOC_PLANE, stranger.id));

        // The guest code grants the same tier it grants at a door.
        lock_op(
            &dc,
            &gc,
            &mut deploys,
            &mut p,
            CX,
            CZ,
            0,
            LOC_PLANE,
            crate::deploy::ACCESS_OP_SET_GUEST,
            4321,
            0,
            &mut ev,
            &mut [ItemStack::default(); INV_SLOTS],
        );
        lock_op(
            &dc,
            &gc,
            &mut deploys,
            &mut stranger,
            CX,
            CZ,
            0,
            LOC_PLANE,
            crate::deploy::ACCESS_OP_ENTER,
            4321,
            1,
            &mut ev,
            &mut [ItemStack::default(); INV_SLOTS],
        );
        assert_eq!(
            (last(&ev).0, last(&ev).2 & 0xff),
            (crate::world::EV_AUTH, crate::lock::GRANT_GUEST as u32),
            "the guest code authorizes at a box exactly as at a door"
        );
        assert!(deploys.lock_passes(CX, CZ, 0, LOC_PLANE, stranger.id));

        // Unlocking reopens it to everyone — a shop's counter chest.
        lock_op(
            &dc,
            &gc,
            &mut deploys,
            &mut p,
            CX,
            CZ,
            0,
            LOC_PLANE,
            crate::deploy::ACCESS_OP_UNLOCK,
            0,
            2,
            &mut ev,
            &mut [ItemStack::default(); INV_SLOTS],
        );
        assert!(!deploys.locks.is_locked(CX, CZ, 0, LOC_PLANE));
        assert!(!deploys.find(CX, CZ, 0, LOC_PLANE).unwrap().locked);
        assert!(deploys.lock_passes(CX, CZ, 0, LOC_PLANE, 424242));
    }

    /// The lock refuses every archetype `lockable` does not name — an
    /// oven is a container with the same shape of address, and the arm
    /// that widened for boxes must not have widened for it.
    #[test]
    fn a_lock_refuses_an_oven_for_support() {
        let dc = boxed_fixture();
        let bc = BuildContent::probe_fixture();
        let mut pieces = Pieces::new();
        let mut deploys = Deploys::new();
        let mut ev = EventQueue::default();
        // A fire (row 4, ground class) on bare terrain at the cell.
        let mut p = player_at_cell(CX, CZ, &[(6, 2), (7, 2)]);
        place_deploy(
            SEED,
            hv(),
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut p,
            0,
            4,
            CX,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(last(&ev).0, crate::world::EV_DEPLOY_PLACED, "fire lands");

        place_deploy(
            SEED,
            hv(),
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut p,
            0,
            5,
            CX,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(
            last(&ev).2,
            REFUSE_D_SUPPORT,
            "a fire is not a thing a lock bolts to"
        );
        assert_eq!(deploys.locks().len(), 0);
    }

    /// The pickup rule a locked door already has, verified at a box: a
    /// locked deployable cannot be lifted by a hand its lock does not
    /// know, and the owner's pickup takes the lock record with the box.
    #[test]
    fn a_lock_guards_the_box_against_being_picked_up() {
        let dc = boxed_fixture();
        let bc = BuildContent::probe_fixture();
        let gc = boxed_gather();
        let mut pieces = Pieces::new();
        let mut deploys = Deploys::new();
        let mut ev = EventQueue::default();
        let mut p = boxed(&bc, &dc, &mut pieces, &mut deploys, &mut ev);
        place_deploy(
            SEED,
            hv(),
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut p,
            0,
            5,
            CX,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
        );
        lock_op(
            &dc,
            &gc,
            &mut deploys,
            &mut p,
            CX,
            CZ,
            0,
            LOC_PLANE,
            crate::deploy::ACCESS_OP_SET_CODE,
            1234,
            0,
            &mut ev,
            &mut [ItemStack::default(); INV_SLOTS],
        );

        let mut stranger = player_at_cell(CX, CZ, &[]);
        stranger.id = 9;
        pick_up(
            &dc,
            &gc,
            &mut pieces,
            &mut deploys,
            &mut stranger,
            CX,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
            &mut [ItemStack::default(); INV_SLOTS],
        );
        assert_eq!(
            (last(&ev).0, last(&ev).2),
            (crate::world::EV_DEPLOY_REFUSED, REFUSE_D_OWNER),
            "a locked box cannot be lifted out from under its lock"
        );
        assert_eq!(deploys.len(), 1, "the box is still standing");

        // The hand the lock knows lifts it, empty, and the lock comes up
        // as a second item; the lock record dies with the box rather than
        // haunting the address for the next thing built there.
        let locks_held = crate::craft::inv_count(&p.inv, 7);
        let boxes_held = crate::craft::inv_count(&p.inv, 9);
        pick_up(
            &dc,
            &gc,
            &mut pieces,
            &mut deploys,
            &mut p,
            CX,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
            &mut [ItemStack::default(); INV_SLOTS],
        );
        assert_eq!(deploys.len(), 0);
        assert_eq!(deploys.locks().len(), 0, "the lock record came off with it");
        assert_eq!(crate::craft::inv_count(&p.inv, 7), locks_held + 1);
        assert_eq!(crate::craft::inv_count(&p.inv, 9), boxes_held + 1);
    }

    /// The guest tier stops at the door verb (`DOORS.md` §2.2, Devblog
    /// 149: open and close, no unlock, no code change, no taking the
    /// lock off) — so a guest must not lift a locked door out of its
    /// frame, which would pocket the lock: `ACCESS_OP_TAKE` wearing the
    /// pickup verb's clothes.
    ///
    /// **Mutant-killer, both directions.** Soften `pick_up`'s check back
    /// to the door-tier `Locks::passes` and the guest's lift lands — the
    /// refusal assertion goes red with the door out of the world and the
    /// lock in the guest's pocket. Harden `use_door`'s check up to
    /// `Locks::passes_full` and the guest's press knocks instead of
    /// swinging — the EV_DOOR assertions go red. The second direction
    /// shipped once, behind a suite that asked only the predicate, which
    /// is why this test drives the verb.
    #[test]
    fn a_guest_works_a_locked_door_and_cannot_pocket_it() {
        let dc = DeployContent::probe_fixture();
        let bc = BuildContent::probe_fixture();
        let gc = GatherContent::probe_fixture();
        let mut pieces = Pieces::new();
        let mut deploys = Deploys::new();
        let mut ev = EventQueue::default();
        let mut p = doored(&bc, &dc, &mut pieces, &mut deploys, &mut ev);
        place_deploy(
            SEED,
            hv(),
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut p,
            0,
            5,
            CX,
            CZ,
            0,
            LOC_EDGE_XLO,
            &mut ev,
        );
        locked_door(&dc, &mut deploys, &mut p, 1234, &mut ev);
        lock_op(
            &dc,
            &gc,
            &mut deploys,
            &mut p,
            CX,
            CZ,
            0,
            LOC_EDGE_XLO,
            crate::deploy::ACCESS_OP_SET_GUEST,
            4321,
            0,
            &mut ev,
            &mut [ItemStack::default(); INV_SLOTS],
        );

        let mut guest = player_at_cell(CX, CZ, &[]);
        guest.id = 9;
        lock_op(
            &dc,
            &gc,
            &mut deploys,
            &mut guest,
            CX,
            CZ,
            0,
            LOC_EDGE_XLO,
            crate::deploy::ACCESS_OP_ENTER,
            4321,
            1,
            &mut ev,
            &mut [ItemStack::default(); INV_SLOTS],
        );
        assert_eq!(
            (last(&ev).0, last(&ev).2 & 0xff),
            (crate::world::EV_AUTH, crate::lock::GRANT_GUEST as u32),
            "the fixture minted a guest, not a member"
        );
        // The guest's one verb, driven through the VERB and not the
        // predicate: a `lock_passes` assert here once stayed green while
        // `use_door` itself asked the wrong tier. Open, then close — the
        // grant covers both presses (`DOORS.md` §1 fact 3).
        use_door(
            &dc,
            &mut pieces,
            &mut deploys,
            &mut guest,
            CX,
            CZ,
            0,
            LOC_EDGE_XLO,
            &mut ev,
        );
        assert_eq!(
            (last(&ev).0, last(&ev).2 & 1, last(&ev).3),
            (crate::world::EV_DOOR, 1, guest.id),
            "a GUEST code opens the locked door — the tier's whole verb"
        );
        assert!(
            deploys.find(CX, CZ, 0, LOC_EDGE_XLO).unwrap().open,
            "and the leaf actually swung"
        );
        use_door(
            &dc,
            &mut pieces,
            &mut deploys,
            &mut guest,
            CX,
            CZ,
            0,
            LOC_EDGE_XLO,
            &mut ev,
        );
        assert_eq!(
            (last(&ev).0, last(&ev).2 & 1),
            (crate::world::EV_DOOR, 0),
            "and closes it again — open and close alike"
        );
        assert!(!deploys.find(CX, CZ, 0, LOC_EDGE_XLO).unwrap().open);

        pick_up(
            &dc,
            &gc,
            &mut pieces,
            &mut deploys,
            &mut guest,
            CX,
            CZ,
            0,
            LOC_EDGE_XLO,
            &mut ev,
            &mut [ItemStack::default(); INV_SLOTS],
        );
        assert_eq!(
            (last(&ev).0, last(&ev).2),
            (crate::world::EV_DEPLOY_REFUSED, REFUSE_D_OWNER),
            "a guest may work the door and nothing else — lifting it \
             would pocket the lock"
        );
        assert_eq!(deploys.len(), 1, "the door is still hanging");
        assert_eq!(deploys.locks().len(), 1, "the lock is still on it");
        assert_eq!(crate::craft::inv_count(&guest.inv, 4), 0);
        assert_eq!(crate::craft::inv_count(&guest.inv, 7), 0);

        // A hand that entered the MAIN code holds full rights and lifts
        // it — the strengthening is a tier, not an owner check.
        let mut friend = player_at_cell(CX, CZ, &[]);
        friend.id = 11;
        lock_op(
            &dc,
            &gc,
            &mut deploys,
            &mut friend,
            CX,
            CZ,
            0,
            LOC_EDGE_XLO,
            crate::deploy::ACCESS_OP_ENTER,
            1234,
            2,
            &mut ev,
            &mut [ItemStack::default(); INV_SLOTS],
        );
        pick_up(
            &dc,
            &gc,
            &mut pieces,
            &mut deploys,
            &mut friend,
            CX,
            CZ,
            0,
            LOC_EDGE_XLO,
            &mut ev,
            &mut [ItemStack::default(); INV_SLOTS],
        );
        assert_eq!(deploys.len(), 0, "a full member lifts it");
        assert_eq!(deploys.locks().len(), 0);
        assert_eq!(crate::craft::inv_count(&friend.inv, 4), 1, "the door");
        assert_eq!(crate::craft::inv_count(&friend.inv, 7), 1, "the lock");
    }

    /// The same wall at the box's plane address — and the boundary the
    /// stronger tier must not have moved: **unlocked stays anyone's**
    /// (demolish v1's landed rule, `DOORS.md` §5).
    #[test]
    fn a_guest_cannot_pocket_a_locked_box_and_unlocked_stays_anyones() {
        let dc = boxed_fixture();
        let bc = BuildContent::probe_fixture();
        let gc = boxed_gather();
        let mut pieces = Pieces::new();
        let mut deploys = Deploys::new();
        let mut ev = EventQueue::default();
        let mut p = boxed(&bc, &dc, &mut pieces, &mut deploys, &mut ev);
        place_deploy(
            SEED,
            hv(),
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut p,
            0,
            5,
            CX,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
        );
        lock_op(
            &dc,
            &gc,
            &mut deploys,
            &mut p,
            CX,
            CZ,
            0,
            LOC_PLANE,
            crate::deploy::ACCESS_OP_SET_CODE,
            1234,
            0,
            &mut ev,
            &mut [ItemStack::default(); INV_SLOTS],
        );
        lock_op(
            &dc,
            &gc,
            &mut deploys,
            &mut p,
            CX,
            CZ,
            0,
            LOC_PLANE,
            crate::deploy::ACCESS_OP_SET_GUEST,
            4321,
            0,
            &mut ev,
            &mut [ItemStack::default(); INV_SLOTS],
        );

        let mut guest = player_at_cell(CX, CZ, &[]);
        guest.id = 9;
        lock_op(
            &dc,
            &gc,
            &mut deploys,
            &mut guest,
            CX,
            CZ,
            0,
            LOC_PLANE,
            crate::deploy::ACCESS_OP_ENTER,
            4321,
            1,
            &mut ev,
            &mut [ItemStack::default(); INV_SLOTS],
        );
        assert!(
            deploys.lock_passes(CX, CZ, 0, LOC_PLANE, guest.id),
            "the lid answers the guest"
        );
        pick_up(
            &dc,
            &gc,
            &mut pieces,
            &mut deploys,
            &mut guest,
            CX,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
            &mut [ItemStack::default(); INV_SLOTS],
        );
        assert_eq!(
            (last(&ev).0, last(&ev).2),
            (crate::world::EV_DEPLOY_REFUSED, REFUSE_D_OWNER),
            "the guest code opens the lid, never the ground under the box"
        );
        assert_eq!(deploys.len(), 1, "the box is still standing");
        assert_eq!(deploys.locks().len(), 1);

        // The owner unlocks — the shop-front state — and a hand with no
        // grant at all lifts it, box and lock both.
        lock_op(
            &dc,
            &gc,
            &mut deploys,
            &mut p,
            CX,
            CZ,
            0,
            LOC_PLANE,
            crate::deploy::ACCESS_OP_UNLOCK,
            0,
            2,
            &mut ev,
            &mut [ItemStack::default(); INV_SLOTS],
        );
        let mut nobody = player_at_cell(CX, CZ, &[]);
        nobody.id = 21;
        pick_up(
            &dc,
            &gc,
            &mut pieces,
            &mut deploys,
            &mut nobody,
            CX,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
            &mut [ItemStack::default(); INV_SLOTS],
        );
        assert_eq!(
            deploys.len(),
            0,
            "an unlocked box is anyone's, lifting included"
        );
        assert_eq!(deploys.locks().len(), 0);
        assert_eq!(crate::craft::inv_count(&nobody.inv, 9), 1, "the box");
        assert_eq!(crate::craft::inv_count(&nobody.inv, 7), 1, "the lock");
    }

    /// Upkeep/decay v1: a half-stocked hearth does half the job, and an
    /// unpaid piece rots at its own material's rate.
    #[test]
    fn upkeep_is_charged_per_material_and_decay_follows_the_grade() {
        let dc = DeployContent::probe_fixture();
        let bc = BuildContent::probe_fixture();
        let mut pieces = Pieces::new();
        let mut deploys = Deploys::new();
        let mut ev = EventQueue::default();
        let mut p = player_at_cell(CX, CZ, &[(0, 250), (1, 250), (2, 2)]);

        // Two pieces at the same cell: the foundation (row 0, item 0 =
        // "wood") and a floor one storey up (row 2, item 1 = "stone" in
        // the build fixture). One hearth covering both.
        founded_graded(&bc, &mut pieces, &mut p, CX, CZ);
        crate::build::place(
            SEED,
            hv(),
            &bc,
            &deploys,
            &mut pieces,
            &mut p,
            0,
            1,
            CX,
            CZ,
            0,
            LOC_EDGE_XLO,
            &mut ev,
        );
        crate::build::place(
            SEED,
            hv(),
            &bc,
            &deploys,
            &mut pieces,
            &mut p,
            0,
            2,
            CX,
            CZ,
            1,
            LOC_PLANE,
            &mut ev,
        );
        // Commit both to stone. The foundation is already there
        // (`founded_graded`, row 5, item 0); the wall and the floor climb
        // to rows 4 and 6, which are priced in item 1. That split — one
        // graded material stocked, one not — is what this test is about,
        // and since twig v0 it cannot be told with scaffold on either
        // side: twig is never charged, so it would rot under any stock at
        // all and prove nothing about the per-row rule.
        for (level, loc) in [(0u8, LOC_EDGE_XLO), (1u8, LOC_PLANE)] {
            crate::build::upgrade(
                &bc,
                &deploys,
                &mut pieces,
                &mut p,
                CX,
                CZ,
                level,
                loc,
                crate::build::MAT_STONE,
                &mut ev,
            );
        }
        place_deploy(
            SEED,
            hv(),
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut p,
            0,
            0,
            CX,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(deploys.hearths().len(), 1);

        // Stock material row 0 only. Under the OLD rule this hearth paid
        // for nothing, because no piece's whole charge was covered.
        deploys.hearths_mut()[0].stock[0] = 100_000;
        deploys.hearths_mut()[0].stock[1] = 0;

        let hp_before: Vec<u16> = pieces.entries().iter().map(|r| r.hp).collect();
        let rows: Vec<u8> = pieces.entries().iter().map(|r| r.row).collect();
        let mut cursor_p = 0u32;
        let mut cursor_d = 0u32;
        let mut budget = 64usize;
        upkeep_sweep(
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            UPKEEP_PERIOD_TICKS,
            &mut cursor_p,
            &mut cursor_d,
            &mut budget,
            &mut ev,
        );
        let hp_after: Vec<u16> = pieces.entries().iter().map(|r| r.hp).collect();

        // At least one piece was paid for and at least one was not: that
        // pair is the whole feature, and asserting "some rotted, some did
        // not" is what a single flat rate could never produce from one
        // half-stocked hearth.
        let intact = hp_before
            .iter()
            .zip(&hp_after)
            .filter(|(a, b)| a == b)
            .count();
        let rotted = hp_before.len() - intact;
        assert!(
            intact > 0 && rotted > 0,
            "a hearth stocked with one material must protect the parts made \
             of it and no others — got {intact} intact, {rotted} rotted"
        );
        assert!(
            deploys.hearths()[0].stock[0] < 100_000,
            "and it must actually have spent the material it had"
        );

        // The ladder: the fixture prices twig 100 / wood 34 / stone 20 /
        // metal 13, so a wooden piece loses more of its max hp per period
        // than a stone one. Read off the content rather than typed, so a
        // re-price moves the assertion with it.
        let twig = decay_at(100, piece_decay_pct(&dc, crate::build::MAT_TWIG));
        let wood = decay_at(100, piece_decay_pct(&dc, crate::build::MAT_WOOD));
        let stone = decay_at(100, piece_decay_pct(&dc, crate::build::MAT_STONE));
        let metal = decay_at(100, piece_decay_pct(&dc, crate::build::MAT_METAL));
        assert!(
            twig > wood && wood > stone && stone > metal,
            "the tougher the grade the slower it rots ({twig}/{wood}/{stone}/{metal})"
        );
        assert_eq!(twig, 100, "a scaffold is gone in one period, not two");
        let _ = rows;
    }

    /// Content that prices no ladder plays the game it played before.
    #[test]
    fn an_unpriced_ladder_falls_back_to_the_flat_rate() {
        let mut dc = DeployContent::probe_fixture();
        dc.decay_pct = [0; DECAY_MATERIALS];
        for m in [
            crate::build::MAT_WOOD,
            crate::build::MAT_STONE,
            crate::build::MAT_METAL,
        ] {
            assert_eq!(
                piece_decay_pct(&dc, m),
                DECAY_PCT_PER_PERIOD,
                "a new table may add a rule, never silently change one"
            );
        }
        // ...and a rate that would round to nothing still subtracts.
        assert_eq!(decay_at(50, 1), 1, "a decay that never subtracts is off");
    }

    /// A hearth with a crew is a base two people can build in — the whole
    /// of hearth crew v1, and the thing `foreign_claim` could not say
    /// before it (`reference/BUILDING.md` §9.1).
    #[test]
    fn a_hearth_answers_to_its_crew_and_not_to_one_id() {
        let dc = DeployContent::probe_fixture();
        let bc = BuildContent::probe_fixture();
        let mut pieces = Pieces::new();
        let mut deploys = Deploys::new();
        let mut ev = EventQueue::default();
        let mut owner = player_at_cell(CX, CZ, &[(0, 99), (1, 99), (2, 2), (3, 2)]);
        founded(&bc, &mut pieces, &mut owner, CX, CZ);
        place_deploy(
            SEED,
            hv(),
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut owner,
            0,
            0,
            CX,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(deploys.hearths().len(), 1, "the hearth stands");
        assert_eq!(
            deploys.hearths()[0].crew.members(),
            &[owner.id],
            "placing it joins its crew — there is no separate authorize step"
        );

        // A stranger in reach is refused by the claim, exactly as before.
        let mut friend = player_at_cell(CX, CZ, &[]);
        friend.id = 9;
        let (ax, az) = cell_center(CX, CZ);
        assert!(
            deploys.foreign_claim(ax, az, friend.id),
            "an outsider is outside"
        );
        assert!(!deploys.foreign_claim(ax, az, owner.id));

        // ...and may not join a crewed hearth on their own say-so. This
        // is the check that keeps a claim a claim.
        crew_op(
            &mut deploys,
            &friend,
            CX,
            CZ,
            0,
            ACCESS_OP_CREW_JOIN,
            &mut ev,
        );
        assert_eq!(
            (last(&ev).0, last(&ev).2),
            (crate::world::EV_DEPLOY_REFUSED, REFUSE_D_OWNER),
            "a stranger cannot authorize themselves into somebody's base"
        );
        assert!(deploys.foreign_claim(ax, az, friend.id));
    }

    /// The three crew ops, their refusals, and the one grant event.
    #[test]
    fn the_crew_ops_join_leave_and_clear() {
        let dc = DeployContent::probe_fixture();
        let bc = BuildContent::probe_fixture();
        let mut pieces = Pieces::new();
        let mut deploys = Deploys::new();
        let mut ev = EventQueue::default();
        let mut owner = player_at_cell(CX, CZ, &[(0, 99), (1, 99), (2, 2), (3, 2)]);
        founded(&bc, &mut pieces, &mut owner, CX, CZ);
        place_deploy(
            SEED,
            hv(),
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut owner,
            0,
            0,
            CX,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
        );
        let (ax, az) = cell_center(CX, CZ);

        // The owner leaves. Nothing gates a leave — a refusal there would
        // tell a stranger whether they were on the list.
        crew_op(
            &mut deploys,
            &owner,
            CX,
            CZ,
            0,
            ACCESS_OP_CREW_LEAVE,
            &mut ev,
        );
        assert!(deploys.hearths()[0].crew.is_empty());
        assert_eq!(
            (last(&ev).0, last(&ev).2 & 0xff),
            (crate::world::EV_AUTH, crate::lock::GRANT_NONE as u32),
            "leaving announces the sender's own standing, absolute"
        );
        assert!(
            deploys.foreign_claim(ax, az, owner.id),
            "and off the crew, even the placer is outside their own claim"
        );

        // An empty crew is anyone's — the same rule a bare door keeps.
        let mut friend = player_at_cell(CX, CZ, &[]);
        friend.id = 9;
        crew_op(
            &mut deploys,
            &friend,
            CX,
            CZ,
            0,
            ACCESS_OP_CREW_JOIN,
            &mut ev,
        );
        assert_eq!(
            (last(&ev).0, last(&ev).2 & 0xff, last(&ev).3),
            (
                crate::world::EV_AUTH,
                crate::lock::GRANT_FULL as u32,
                friend.id
            ),
            "the grant is the sender's own fact"
        );
        assert!(!deploys.foreign_claim(ax, az, friend.id));

        // Now crewed, the owner is the outsider and must be let back in by
        // somebody who is on it. Joining twice is not a bug.
        crew_op(
            &mut deploys,
            &owner,
            CX,
            CZ,
            0,
            ACCESS_OP_CREW_JOIN,
            &mut ev,
        );
        assert_eq!(last(&ev).2, REFUSE_D_OWNER);
        deploys.hearths_mut()[0].crew.add(owner.id);
        crew_op(
            &mut deploys,
            &owner,
            CX,
            CZ,
            0,
            ACCESS_OP_CREW_JOIN,
            &mut ev,
        );
        assert_eq!(last(&ev).0, crate::world::EV_AUTH, "a double press is fine");
        assert_eq!(deploys.hearths()[0].crew.len(), 2);

        // Clear takes it back to the clearer alone, in one step — never
        // through an empty crew, which anyone could have joined.
        crew_op(
            &mut deploys,
            &owner,
            CX,
            CZ,
            0,
            ACCESS_OP_CREW_CLEAR,
            &mut ev,
        );
        assert_eq!(deploys.hearths()[0].crew.members(), &[owner.id]);
        assert!(
            deploys.foreign_claim(ax, az, friend.id),
            "the friend is out"
        );

        // The two shape refusals: no hearth there, and out of reach.
        crew_op(
            &mut deploys,
            &owner,
            CX + 3,
            CZ,
            0,
            ACCESS_OP_CREW_JOIN,
            &mut ev,
        );
        assert_eq!(last(&ev).2, REFUSE_D_HEARTH, "no hearth at that address");
        let far = player_at_cell(CX + 7, CZ, &[]);
        crew_op(&mut deploys, &far, CX, CZ, 0, ACCESS_OP_CREW_JOIN, &mut ev);
        assert_eq!(last(&ev).2, REFUSE_D_REACH, "a crew op has the build reach");
        assert_eq!(deploys.hearths()[0].crew.len(), 1);
    }

    /// A crew is what lets a second pair of hands build in the base, and
    /// this is that sentence as the three verbs it actually gates.
    #[test]
    fn a_crewmate_may_build_upgrade_and_deploy_inside_the_claim() {
        let dc = DeployContent::probe_fixture();
        let bc = BuildContent::probe_fixture();
        let mut pieces = Pieces::new();
        let mut deploys = Deploys::new();
        let mut ev = EventQueue::default();
        let mut owner = player_at_cell(CX, CZ, &[(0, 99), (1, 99), (2, 2), (3, 2)]);
        founded(&bc, &mut pieces, &mut owner, CX, CZ);
        place_deploy(
            SEED,
            hv(),
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut owner,
            0,
            0,
            CX,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
        );

        let mut friend = player_at_cell(CX, CZ, &[(0, 99), (1, 99), (3, 2)]);
        friend.id = 9;
        // Outside the crew: the wall refuses.
        crate::build::place(
            SEED,
            hv(),
            &bc,
            &deploys,
            &mut pieces,
            &mut friend,
            0,
            1,
            CX,
            CZ,
            0,
            LOC_EDGE_ZLO,
            &mut ev,
        );
        assert_eq!(
            (last(&ev).0, last(&ev).2),
            (crate::world::EV_BUILD_REFUSED, crate::build::REFUSE_B_CLAIM)
        );

        // On it: the same wall lands.
        deploys.hearths_mut()[0].crew.add(friend.id);
        crate::build::place(
            SEED,
            hv(),
            &bc,
            &deploys,
            &mut pieces,
            &mut friend,
            0,
            1,
            CX,
            CZ,
            0,
            LOC_EDGE_ZLO,
            &mut ev,
        );
        assert_eq!(
            last(&ev).0,
            crate::world::EV_PIECE_PLACED,
            "a crewmate builds in the base"
        );
        // ...and the deploy verb gates on the same predicate. Asserted as
        // "the claim stopped being the reason" rather than as a landing,
        // because a landing would also be asserting this fixture's
        // terrain and support, which are not what a crew changes.
        let mut outsider = player_at_cell(CX, CZ, &[(3, 2)]);
        outsider.id = 11;
        place_deploy(
            SEED,
            hv(),
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut outsider,
            0,
            1,
            CX,
            CZ,
            1,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(
            (last(&ev).0, last(&ev).2),
            (crate::world::EV_DEPLOY_REFUSED, REFUSE_D_CLAIM),
            "an outsider's deploy is refused BY THE CLAIM"
        );
        deploys.hearths_mut()[0].crew.add(outsider.id);
        place_deploy(
            SEED,
            hv(),
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut outsider,
            0,
            1,
            CX,
            CZ,
            1,
            LOC_PLANE,
            &mut ev,
        );
        assert_ne!(
            last(&ev).2,
            REFUSE_D_CLAIM,
            "and on the crew the claim is no longer what refuses it"
        );
    }

    /// A lock dies with the door it is bolted to — the one removal path.
    #[test]
    fn a_removed_door_takes_its_lock_with_it() {
        let dc = DeployContent::probe_fixture();
        let bc = BuildContent::probe_fixture();
        let mut pieces = Pieces::new();
        let mut deploys = Deploys::new();
        let mut ev = EventQueue::default();
        let mut p = doored(&bc, &dc, &mut pieces, &mut deploys, &mut ev);
        place_deploy(
            SEED,
            hv(),
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut p,
            0,
            5,
            CX,
            CZ,
            0,
            LOC_EDGE_XLO,
            &mut ev,
        );
        locked_door(&dc, &mut deploys, &mut p, 1234, &mut ev);
        assert_eq!(deploys.locks().len(), 1);

        let di = deploys
            .find_index(CX, CZ, 0, LOC_EDGE_XLO)
            .expect("the door");
        drop_deploy(&dc, &mut pieces, &mut deploys, di, &mut ev);
        assert_eq!(
            deploys.locks().len(),
            0,
            "a lock left at a dead address would refuse the next door built there"
        );
    }

    #[test]
    fn use_refusals_name_no_door_and_reach() {
        let dc = DeployContent::probe_fixture();
        let bc = BuildContent::probe_fixture();
        let mut pieces = Pieces::new();
        let mut deploys = Deploys::new();
        let mut ev = EventQueue::default();

        // Nothing at the address.
        let mut p = player_at_cell(CX, CZ, &[(0, 99), (1, 99), (3, 1), (4, 2)]);
        use_door(
            &dc,
            &mut pieces,
            &mut deploys,
            &mut p,
            CX,
            CZ,
            0,
            LOC_EDGE_XLO,
            &mut ev,
        );
        assert_eq!(last(&ev).2, REFUSE_D_DOOR);

        // A deployable that is not a door (the any-class workbench).
        founded(&bc, &mut pieces, &mut p, CX, CZ);
        place_deploy(
            SEED,
            hv(),
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut p,
            0,
            1,
            CX,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(last(&ev).0, crate::world::EV_DEPLOY_PLACED);
        use_door(
            &dc,
            &mut pieces,
            &mut deploys,
            &mut p,
            CX,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(last(&ev).2, REFUSE_D_DOOR, "a workbench is not a door");

        // A real door, toggled from 20 m away: reach refusal, state kept.
        crate::build::place(
            SEED,
            hv(),
            &bc,
            &deploys,
            &mut pieces,
            &mut p,
            0,
            3,
            CX,
            CZ,
            0,
            LOC_EDGE_XLO,
            &mut ev,
        );
        place_deploy(
            SEED,
            hv(),
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut p,
            0,
            2,
            CX,
            CZ,
            0,
            LOC_EDGE_XLO,
            &mut ev,
        );
        assert_eq!(last(&ev).0, crate::world::EV_DEPLOY_PLACED);
        let mut far = player_at_cell(CX + 7, CZ, &[]);
        use_door(
            &dc,
            &mut pieces,
            &mut deploys,
            &mut far,
            CX,
            CZ,
            0,
            LOC_EDGE_XLO,
            &mut ev,
        );
        assert_eq!(last(&ev).2, REFUSE_D_REACH);
        assert!(!deploys.find(CX, CZ, 0, LOC_EDGE_XLO).unwrap().open);
        assert_eq!(pieces.cols().get(CX, CZ).shut_xlo, 1);
    }

    #[test]
    fn doorway_decay_takes_the_door_and_unseals() {
        let dc = DeployContent::probe_fixture();
        let bc = BuildContent::probe_fixture();
        let mut pieces = Pieces::new();
        let mut deploys = Deploys::new();
        let mut ev = EventQueue::default();
        doored(&bc, &dc, &mut pieces, &mut deploys, &mut ev);
        assert_eq!(pieces.cols().get(CX, CZ).shut_xlo, 1);

        // No hearth anywhere: everything decays away within 20 periods.
        let mut pc = 0u32;
        let mut dcur = 0u32;
        for hour in 1..=20u64 {
            upkeep_sweep(
                &dc,
                &bc,
                &mut pieces,
                &mut deploys,
                hour * UPKEEP_PERIOD_TICKS + 1,
                &mut pc,
                &mut dcur,
                &mut tick_budget(),
                &mut ev,
            );
            if pieces.is_empty() && deploys.is_empty() {
                break;
            }
        }
        assert!(pieces.is_empty(), "unpaid pieces never decayed away");
        assert!(
            deploys.is_empty(),
            "the door must go with (or before) its doorway"
        );
        let m = pieces.cols().get(CX, CZ);
        assert_eq!(
            (m.doors_xlo, m.shut_xlo),
            (0, 0),
            "a decayed doorway leaves no door collision behind"
        );
    }

    /// The box store's cap, and its stated overflow policy: **refuse**
    /// (wall 4). Written against the private `len` rather than by placing
    /// two hundred and fifty-six boxes, because the thing under test is
    /// the guard and not the terrain generator's supply of flat cells.
    ///
    /// The refusal has to land *before* the deploy record is inserted, or
    /// a full box store would leave a standing box with nowhere to keep
    /// what you put in it — a container that silently eats items, which is
    /// the one failure mode worse than refusing to place.
    #[test]
    fn a_full_box_store_refuses_the_placement_and_places_nothing() {
        let mut dc = DeployContent::probe_fixture();
        dc.defs[4] = DeployDef {
            arch: ARCH_BOX,
            placement: PLACE_FOUNDATION,
            hp: 100,
            item: 6,
            ..DeployDef::INERT
        };
        dc.def_count = 5;
        let bc = BuildContent::probe_fixture();
        let mut pieces = Pieces::new();
        let mut deploys = Deploys::new();
        let mut ev = EventQueue::default();
        let mut p = player_at_cell(CX, CZ, &[(0, 9), (6, 4)]);
        founded(&bc, &mut pieces, &mut p, CX, CZ);

        deploys.boxes.len = MAX_BOXES;
        let deploys_before = deploys.len();
        let held = crate::craft::inv_count(&p.inv, 6);
        ev.clear();
        place_deploy(
            SEED,
            hv(),
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut p,
            0,
            4,
            CX,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(last(&ev).0, crate::world::EV_DEPLOY_REFUSED);
        assert_eq!(last(&ev).2, REFUSE_D_FULL, "a full box store refuses");
        assert_eq!(
            deploys.len(),
            deploys_before,
            "a refused box must not leave a deploy record standing"
        );
        assert_eq!(
            crate::craft::inv_count(&p.inv, 6),
            held,
            "a refused placement must not spend the item"
        );

        // One slot back and the same placement succeeds — a guard that
        // refused unconditionally would pass every assert above.
        deploys.boxes.len = MAX_BOXES - 1;
        ev.clear();
        place_deploy(
            SEED,
            hv(),
            &dc,
            &bc,
            &mut pieces,
            &mut deploys,
            &mut p,
            0,
            4,
            CX,
            CZ,
            0,
            LOC_PLANE,
            &mut ev,
        );
        assert_eq!(last(&ev).0, crate::world::EV_DEPLOY_PLACED);
        assert_eq!(
            deploys.boxes.len, MAX_BOXES,
            "the record took the last slot"
        );
    }
}
