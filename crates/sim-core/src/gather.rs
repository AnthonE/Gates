//! Gather — the first survival verb (DESIGN.md §2, M1). A swing at a
//! standing scatter slot pays yield into the swinger's inventory; enough
//! hits exhaust the slot until a jittered respawn (TERRAIN.md §2: the
//! server owns one bit + one timer per slot — this module is that bit and
//! that timer). Pure and fixed-capacity like everything else in the crate.
//!
//! Content reaches the sim only as a baked `GatherContent` table — the
//! shard bakes it from `content/*.toml` at boot (CLAUDE.md wall 7); the
//! inert `EMPTY` default makes gather a no-op, and `probe_fixture()` is a
//! synthetic table for the parity/replay/alloc gates (fixture, not game
//! content — real numbers never live in code).
//!
//! Verb constants below are proposed defaults, DECISIONS.md §open
//! ("gather verb v0" / "gather bounds & overflow policies" rows).
//! Respawn window is the spoken §open row "node/barrel respawn 20–45 min".

use crate::input::BTN_PRIMARY;
use crate::limits::{INV_SLOTS, MAX_ITEM_DEFS, MAX_SLOT_LIVES};
use crate::loot::{LootContent, LOOT_BARREL};
use crate::movement::{quant_xz, quant_y, POS_XZ_Q, POS_Y_Q};
use crate::rng::{cell_hash, splitmix64};
use crate::terrain::{self, Occupant, ScatterTable, CELL_SIZE};
use crate::world::{
    EventQueue, Player, EV_GATHER, EV_GATHER_REFUSED, EV_SLOT_HARVESTED, EV_WEAK_MARK,
};
use crate::yaw_lut::yaw_dir;

/// Sentinel: no item. Doubles as the bare-hand "held item".
pub const NO_ITEM: u16 = u16::MAX;

/// Why a gather swing was refused — the low half of `EV_GATHER_REFUSED.b`.
/// Zero is reserved as "no reason" (the consume ledger's posture), so the
/// encoder can refuse it the way `encode_event_consume_refused` refuses a
/// zero reason.
///
/// The node pays nothing for what is in the hand — a torch at a tree, a
/// spear at a stone node, a bare fist at anything swung. The event's high
/// half names the held item (`NO_ITEM` = bare hands) so the client can say
/// *a torch cannot fell a tree* instead of "bare hands" (`NOW.md` §0kit
/// item 2: hotbar 2 is one key from the rock).
pub const REFUSE_G_TOOL: u32 = 1;
/// The held item is a tool for this node and it is **dead** — condition
/// zero with a nonzero ceiling. It stays in the hand and stops being a
/// tool (Q4, operator 2026-08-15); the fix the sentence names is a
/// re-craft, because re-craft is the repair (Q3).
pub const REFUSE_G_BROKEN: u32 = 2;
/// The highest live reason, named rather than counted — the ledger the
/// wire's field width is bounded against (`protocol::event`).
pub const REFUSE_G_MAX: u32 = REFUSE_G_BROKEN;

/// Sentinel cell key: no weak-spot chase in progress (`Player::ws_cell`).
pub const NO_CELL: u32 = u32::MAX;

/// Occupants that can be gathered: Tree, StoneNode, MetalNode, SulfurNode,
/// Bush — terrain `Occupant` 1..=5. Rock is not a node and never will be.
///
/// BarrelSlot is not a node either, and is now swingable anyway: it takes
/// hits and exhausts on the same `SlotLives` bit and the same respawn
/// timer, but it has no `NodeDef`, no per-tool yield, no weak spot, and it
/// pays nothing into the swinger's hands. It comes apart into a container
/// (`Swing::Smashed` → `loot.rs`), which is the difference between a tree
/// and a barrel: a tree is a resource and a barrel is a reward.
pub const GATHERABLE_KINDS: usize = 5;

/// Scan-target index for a barrel slot — one past the gatherable range,
/// so the 3×3 scan ranks nodes and barrels against each other by distance
/// with one comparison. One arm, one target: a tree standing nearer than
/// a barrel still wins the swing.
const BARREL_TARGET: usize = GATHERABLE_KINDS;

/// Tool rows one node archetype can carry (alpha data uses ≤ 4 + hand;
/// bake refuses past this). Structural cap, not a knob.
pub const MAX_TOOLS_PER_NODE: usize = 8;

/// Ticks between swings while the primary button is held: ~47 swings/min
/// at 30 Hz, the melee-band cadence. Paid per swing, hit or whiff.
pub const SWING_INTERVAL_TICKS: u64 = 38;

/// Budget one unmarked swing spends against a node's pool, and the
/// denominator every proportional payout divides by. A node holds
/// `NodeDef::hits × HIT_UNIT`; `SlotLife::hits` counts budget SPENT, not
/// swings landed, which is what lets a marked swing take a bigger bite
/// without paying more in total (`NodeDef::weak_pct`). 100 so a content
/// percentage lands on it exactly. Structural, not a knob.
pub const HIT_UNIT: u32 = 100;
/// Reach in meters (matches the melee weapon rows' range_m = 2).
pub const REACH_M: f32 = 2.0;
/// Aim cone half-angle 30°: cos authored offline (√3/2), no trig at
/// runtime — same discipline as terrain's CLIFF_SLOPE_RATIO.
pub const CONE_COS: f32 = 0.866_025_4;
/// Vertical acceptance window: slot within ±3 m of the feet. Aim is
/// planar in v0; pitch starts mattering with M2's raycasts.
pub const DY_MAX_M: f32 = 3.0;
/// Standing inside the node (≤ 0.2 m planar) bypasses the cone test —
/// a zero-length aim vector has no direction to test against.
pub const POINT_BLANK_M2: f32 = 0.04;

/// Weak-spot sector half-angle 45°: cos authored offline (√2/2), no trig
/// at runtime. A hit landed while standing inside the mark's sector pays
/// the content's `weak_spot_bonus_pct` extra (DECISIONS.md §open, "gather
/// verb v0").
pub const WEAK_COS: f32 = 0.707_106_77;

/// Node respawn window in ticks: 20–45 min at 30 Hz (DECISIONS.md §open,
/// "node/barrel respawn").
pub const RESPAWN_MIN_TICKS: u64 = 36_000;
pub const RESPAWN_RANGE_TICKS: u64 = 45_000;

/// Noise channel for respawn jitter (worldgen channels live in terrain.rs;
/// this one is sim-side and collides with nothing below 96).
const CH_RESPAWN: u32 = 97;
/// Noise channel for the weak-spot mark heading.
const CH_WEAK: u32 = 98;

/// One gatherable archetype's baked rules. `output == NO_ITEM` ⇒ not
/// gatherable (the inert default).
#[derive(Clone, Copy, Debug)]
pub struct NodeDef {
    /// Item index this node yields.
    pub output: u16,
    /// Unmarked swings to exhaust the node. The node's whole payout is
    /// `hits × yield_for(tool)` however it is struck — see `weak_pct`.
    pub hits: u16,
    /// Units per bare-hand swing.
    pub hand_yield: u16,
    /// Extra **budget** a marked swing consumes, % (content
    /// `weak_spot_bonus_pct`); 0 disables the mark for this archetype.
    ///
    /// **The mark buys speed, not yield** (operator, 2026-08-09; the
    /// reference's own model, `reference/RIPLIST.md` §4.3 — Facepunch:
    /// *"you will not actually earn more resources, but by using skill
    /// and good aim you can harvest the ore faster"*). A node holds
    /// `hits × HIT_UNIT` of budget; an unmarked swing spends `HIT_UNIT`
    /// and a marked one spends `HIT_UNIT + weak_pct`, and pay is
    /// proportional to the budget spent. So the total is invariant and
    /// the skilled player empties the node in fewer swings — where the
    /// old model paid them 1.5× and made them richer instead.
    pub weak_pct: u16,
    /// Share of the node's whole payout withheld from the per-swing pay
    /// and handed over on the swing that exhausts it, % (content
    /// `finish_bonus_pct`); 0 pays evenly.
    ///
    /// The reference's anti-cherry-picking rule (Devblog 166, *"The final
    /// hit will yield a bonus of about 20% of the total, which is not only
    /// satisfying but should mitigate cherry picking"* — their own hedge
    /// on the 20%, so ours is a reading of it, not a taken constant).
    /// Their tree splits the same way at half: *"You now receive half
    /// while harvesting and the other half as a finishing bonus"*
    /// (Devblog **186**, not 187 — the fall is the tell, not the
    /// trigger). It is a *redistribution*, never a bonus on top — a node
    /// abandoned half-struck is worth strictly less per swing than one
    /// finished.
    ///
    /// **Not modelled:** theirs pays this bonus only to a proper tool
    /// (Devblog 166: *"Bone clubs and stones do not trigger it"*), so our
    /// rock finishes a node for full value where theirs pays nothing.
    pub finish_pct: u16,
    /// (item index, units per swing) rows; `(NO_ITEM, 0)` = empty row.
    pub tools: [(u16, u16); MAX_TOOLS_PER_NODE],
    /// (item index, condition loss per landed hit) rows, hundredths of a
    /// point; `(NO_ITEM, 0)` = empty row. Keyed per **(tool, node)**
    /// exactly as `tools` is, because that is the reference's own model
    /// (`reference/DURABILITY.md` §2: a metal hatchet pays 0.3 on a tree
    /// and 1.0 on flesh) — **the table IS the wrong-tool predicate, there
    /// is no predicate to port**. A tool with no row here wears nothing on
    /// this node; content validation (V4) is what guarantees every tool a
    /// node pays has one.
    pub wear: [(u16, u16); MAX_TOOLS_PER_NODE],
    /// A second thing this node pays, flat: `(item index, units per swing)`,
    /// `(NO_ITEM, 0)` for the nodes that pay one thing. Deliberately **not**
    /// tool-scaled and **not** weak-spot bonused — a bush pays its berries
    /// to a bare hand exactly as it pays them to a hatchet, because picking
    /// is not chopping. The primary keeps both, so the tool ladder and the
    /// glint still mean what they meant.
    pub secondary: (u16, u16),
}

impl NodeDef {
    pub const INERT: Self = Self {
        output: NO_ITEM,
        hits: 0,
        hand_yield: 0,
        weak_pct: 0,
        finish_pct: 0,
        tools: [(NO_ITEM, 0); MAX_TOOLS_PER_NODE],
        wear: [(NO_ITEM, 0); MAX_TOOLS_PER_NODE],
        secondary: (NO_ITEM, 0),
    };

    /// Units this node pays per swing of `held` (falls back to the hand
    /// row when the held item has no tool row — punching with wood in
    /// hand is still punching).
    pub fn yield_for(&self, held: u16) -> u16 {
        if held != NO_ITEM {
            for &(item, per_hit) in self.tools.iter() {
                if item == held {
                    return per_hit;
                }
            }
        }
        self.hand_yield
    }

    /// Condition this node takes off one landed hit of `held`, in
    /// hundredths of a point. Zero for a bare hand and for any tool the
    /// wear table has no row for — per **(tool, node)**, never per tool,
    /// which is the whole design (`wear`'s doc).
    pub fn wear_for(&self, held: u16) -> u16 {
        if held != NO_ITEM {
            for &(item, loss) in self.wear.iter() {
                if item == held {
                    return loss;
                }
            }
        }
        0
    }
}

/// The whole gather ruleset the sim knows: per-archetype node rules plus
/// per-item stack ceilings. Construction input like the seed — the WAL
/// pins the content hash it was baked from (CONTENT.md §0).
#[derive(Clone, Copy, Debug)]
pub struct GatherContent {
    /// Indexed by `Occupant as usize - 1` (Tree..Bush).
    pub nodes: [NodeDef; GATHERABLE_KINDS],
    pub stack_max: [u16; MAX_ITEM_DEFS],
    /// Maximum condition per item, hundredths of a point (content
    /// `condition_max`; item durability v0). **Absent means 0 means never
    /// wears and can never be repaired** — the schema default IS the rule
    /// for non-tools, so wood, stone and every consumable sit at 0 and
    /// nothing ever asks about them. A fresh stack of an item with a
    /// nonzero row is minted at this value (`inv_add`'s `cond`), and a
    /// stack of it at `cond == 0` is a **dead tool**: still in the hand,
    /// no longer a tool (`swing`'s Q4 guard).
    pub cond_max: [u16; MAX_ITEM_DEFS],
    pub item_count: u16,
}

impl GatherContent {
    /// The stack ceiling for an item, total over every `u16` — an index
    /// past the table reads as zero, which every caller already treats as
    /// "the ladder cannot size this, so it cannot be carried"
    /// (`backpack.rs`'s loot skip, `inventory.rs`'s `REFUSE_M_UNSTACKABLE`).
    /// The bounds test lived inlined at each call site; one item id arrives
    /// from the wire and one from a WAL, so it wants to be one function.
    pub fn stack_max_of(&self, item: u16) -> u16 {
        if (item as usize) < MAX_ITEM_DEFS {
            self.stack_max[item as usize]
        } else {
            0
        }
    }

    /// The condition ceiling for an item — `stack_max_of`'s shape for
    /// `cond_max`'s reason: one id arrives from the wire and one from a
    /// WAL, and an index past the table reads as "carries no condition",
    /// which is what every caller already does with 0.
    pub fn cond_max_of(&self, item: u16) -> u16 {
        if (item as usize) < MAX_ITEM_DEFS {
            self.cond_max[item as usize]
        } else {
            0
        }
    }

    /// Inert: nothing is gatherable. `World::new` starts here; the boot
    /// path installs the baked table before the first tick.
    pub const EMPTY: Self = Self {
        nodes: [NodeDef::INERT; GATHERABLE_KINDS],
        stack_max: [0; MAX_ITEM_DEFS],
        cond_max: [0; MAX_ITEM_DEFS],
        item_count: 0,
    };

    /// Synthetic table for the parity/replay/alloc gates. Deliberately
    /// unlike game content: outputs double as tools (item 0 gathers item
    /// 1 faster and vice versa) so bot runs cover the tool-yield path the
    /// moment a bot's slot 0 fills. Real values bake from content/*.toml.
    pub fn probe_fixture() -> Self {
        let mut c = Self::EMPTY;
        // Eleven. Every index below is spoken for by a fixture
        // deployable's `item` (hearth 2, workbench 3, door 4, bag 5, fire
        // 6, lock 7, recycler 8, research table 10) plus the box that
        // `deploy.rs`'s `boxed_fixture` appends at 9. Two deployables
        // sharing an item is what `DeployContent::probe_fixture` warns
        // about at the lock row — the give-back hands out the wrong
        // object — so the fixture widens rather than doubling up.
        c.item_count = 11;
        let mut i = 0;
        while i < 11 {
            c.stack_max[i] = 100;
            i += 1;
        }
        // The two items the nodes below use as tools carry condition, so
        // wear rides the parity/replay/alloc surfaces the moment a bot
        // gathers with one — and since gathered stacks are minted at the
        // item's own ceiling (`inv_add`'s `cond`), the tool-yield path
        // this fixture exists for keeps working while it wears. Values
        // distinct from each other and from every stack ceiling, so a
        // transposed field cannot hide (event_roles' discipline 2).
        c.cond_max[0] = 400;
        c.cond_max[1] = 300;
        // (output, hits, hand, weak %, finish %, tool-item, tool-yield,
        // wear-per-hit). Finish shares are deliberately varied and
        // deliberately NOT the shipped content's — a fixture that matches
        // the game hides a bake that ignores the column. Wear rates vary
        // per (tool, node) for the same reason: one rate everywhere hides
        // a `wear_for` that reads the tool and ignores the node.
        type FixtureRow = (u16, u16, u16, u16, u16, u16, u16, u16);
        let rows: [FixtureRow; GATHERABLE_KINDS] = [
            // Tree and Bush withhold nothing, so the mark and side-payout
            // gates read pay directly. Stone and Sulfur carry the finish
            // coverage, at different shares and deliberately NOT the
            // shipped content's — a fixture that matches the game hides a
            // bake that ignores the column.
            (0, 4, 7, 100, 0, 1, 13, 7),     // Tree
            (1, 5, 6, 50, 40, 0, 11, 5),     // StoneNode
            (2, 6, 3, 25, 0, 0, 9, 3),       // MetalNode: pays evenly
            (3, 6, 3, 75, 10, 1, 9, 2),      // SulfurNode
            (4, 1, 10, 0, 0, NO_ITEM, 0, 0), // Bush: one-hit pickup, no mark
        ];
        let mut k = 0;
        while k < GATHERABLE_KINDS {
            let (out, hits, hand, weak, finish, tool, per, wear) = rows[k];
            c.nodes[k] = NodeDef {
                output: out,
                hits,
                hand_yield: hand,
                weak_pct: weak,
                finish_pct: finish,
                tools: [(NO_ITEM, 0); MAX_TOOLS_PER_NODE],
                wear: [(NO_ITEM, 0); MAX_TOOLS_PER_NODE],
                secondary: (NO_ITEM, 0),
            };
            if tool != NO_ITEM {
                c.nodes[k].tools[0] = (tool, per);
                c.nodes[k].wear[0] = (tool, wear);
            }
            k += 1;
        }
        // The bush pays item 0 on the side — the same item
        // `SurvivalContent::probe_fixture` makes food, so the secondary
        // payout is on the walls that run this fixture rather than only in
        // a unit test, and the two fixtures agree about what food is.
        c.nodes[4].secondary = (0, 3);
        c
    }
}

/// Gatherable index of an occupant, or None for Rock/Barrel/None.
#[inline]
pub fn node_index(o: Occupant) -> Option<usize> {
    let i = o as usize;
    if (1..=GATHERABLE_KINDS).contains(&i) {
        Some(i - 1)
    } else {
        None
    }
}

/// What the 3×3 scan may aim at: a gatherable index, or `BARREL_TARGET`.
/// `None` for Rock and empty cells — the two things a swing passes through.
#[inline]
fn target_index(o: Occupant) -> Option<usize> {
    match node_index(o) {
        Some(ni) => Some(ni),
        None if o == Occupant::BarrelSlot => Some(BARREL_TARGET),
        None => None,
    }
}

/// Terrain occupant ordinal of a scan target — the value
/// `EV_SLOT_HARVESTED` names in field `b`. The event says *what* stopped
/// standing there, not which row of the gather table it came from: a
/// barrel has no row, and "gatherable index" was only ever the occupant
/// ordinal minus one anyway.
#[inline]
fn occupant_of(target: usize) -> u32 {
    if target == BARREL_TARGET {
        Occupant::BarrelSlot as u32
    } else {
        target as u32 + 1
    }
}

/// The 3×3 scan's pick: the nearest swingable slot in reach and inside the
/// aim cone. A named struct rather than a tuple because it grew a seventh
/// member (the slot's own world position, which a smashed barrel needs to
/// stand its container up at) and a seven-tuple is where a positional
/// payload starts going wrong — the exact failure mode `event_roles.rs`
/// exists to catch one layer up.
struct Target {
    /// Planar distance², for the nearest-wins comparison.
    d2: f32,
    /// Slot→player planar offset, for the weak-spot sector test.
    ox: f32,
    oz: f32,
    cx: u16,
    cz: u16,
    /// Gatherable index, or `BARREL_TARGET`.
    ni: usize,
    /// The slot's world position (m).
    pos: (f32, f32, f32),
    /// The occupant's own radius and top, **already scaled** by the slot
    /// (`terrain::occupant_volume` × `Slot::scale`) — the same pair
    /// `terrain::slot_blocks` collides against. Carried from the pick
    /// rather than re-queried at the push site because the slot is right
    /// here and a second `cache.slot` call would be a second chance to
    /// disagree with the thing we actually hit.
    r: f32,
    top: f32,
}

/// Where up a short occupant a strike lands, as a fraction of its height.
///
/// **Its own constant because it is an invented number and the rule is that
/// they are spoken** (`CLAUDE.md` §loop discipline; `DECISIONS.md` §open,
/// "melee mark v0"). Half is the middle of the thing rather than a tuning
/// — you cannot strike the centre of a knee-high rock from eye level — but
/// a bare `* 0.5` in an expression is not registrable, and the two
/// constants either side of it in this seam both carry rows.
pub const STRIKE_WAIST_FRAC: f32 = 0.5;

/// A melee strike lands at the swinger's eye height, because melee is
/// planar: the pick below reads `yaw` and never `pitch` (this file says so
/// in words at the top), so the arm swings level from the same origin
/// `ranged::fire` shoots from. Derived from `ARROW_EYE_MM` rather than
/// picked, so the two origins cannot drift apart — no new knob is spoken
/// for here and none is invented.
const EYE_M: f32 = crate::ranged::ARROW_EYE_MM as f32 / 1000.0;

/// Where a landed swing scuffs the thing it hit: the point on the struck
/// occupant's own collision skin, on the side the swinger is standing.
///
/// `None` for an occupant with no volume. The bush is the only swingable
/// one (`terrain::occupant_volume` gives it `(0.0, 0.0)`) and a bundle of
/// leaves has no surface to mark, so it gets no mark rather than a mark at
/// its centre. That refusal is also what keeps the arithmetic safe: a
/// positive radius means the slot blocks, which means the swinger is
/// standing outside it, so `d2` cannot be the zero this function would
/// otherwise divide by.
///
/// The offset direction is slot→swinger, which is exactly what
/// `render/decal.rs::facing` re-derives at the other end — the horizontal
/// from the scatter slot's centre to the impact point. So the decal turns
/// to face whoever made it and no normal ever crosses the wire.
fn skin_point(
    pos: (f32, f32, f32),
    ox: f32,
    oz: f32,
    d2: f32,
    r: f32,
    top: f32,
    strike_y: f32,
) -> Option<(f32, f32, f32)> {
    if r <= 0.0 || top <= 0.0 || d2 <= 0.0 {
        return None;
    }
    let inv = 1.0 / d2.sqrt();
    // **Eye height, or the occupant's waist if the occupant is shorter.**
    //
    // Measured rather than reasoned, and the first cut was wrong: clamping
    // to the occupant's TOP put a mark on the rim of the boulder beside
    // spawn — 13.43 against an eye at 13.87 — where the mark's own normal
    // is horizontal and the surface curves away under it, so a decal
    // projecting sideways across a rounded rim grazes it and draws
    // nothing. A capture aimed at those exact coordinates is what found it.
    //
    // Half the height is not a tuned number, it is the middle of the
    // thing: you cannot strike the centre of a knee-high rock from eye
    // level, and for anything taller than you — every tree — the eye still
    // wins, which is where a swing at a trunk actually lands.
    let y = strike_y.min(pos.1 + top * STRIKE_WAIST_FRAC).max(pos.1);
    Some((pos.0 + ox * inv * r, y, pos.2 + oz * inv * r))
}

/// What a swing did, for the caller that owns the stores gather does not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Swing {
    /// No swing this tick (button up, or still on cooldown), or a node
    /// absorbed it. Either way the arm is not free.
    Absorbed,
    /// A swing was taken and nothing absorbed it — the cadence is paid and
    /// the arm is still moving, so the caller hands it to `combat::strike`.
    Free,
    /// A swing was taken, it was aimed at a gather node, and the node
    /// refused it (a tool it pays nothing for — which since Q4 includes a
    /// dead one). The cadence is paid and the arm is still free **for
    /// flesh**: the caller hands it to `combat::strike` and `mob::strike`
    /// exactly as `Free`, because a node must not become cover
    /// (`tests/gather.rs` `a_refused_gather_swing_leaves_the_arm_free`).
    /// What the caller must NOT do is pass it to `combat::raid` — the
    /// swing was aimed at the node, and `raid` has no owner or privilege
    /// filter, so a stone hatchet aimed at a stone node inside your own
    /// base was chipping your own wall silently (`NOW.md` §0kit item 1;
    /// the fall-through was proven by fixture: piece hp fell at
    /// `hand_yield = 0` and not at 25).
    Refused,
    /// A barrel came apart. The swing is spent; what falls out is the
    /// caller's to roll, because it owns the container store and gather
    /// deliberately does not. Address is the smashed slot's own quantized
    /// position — the sim sims on the values it transmits, so the
    /// container stands exactly where the client drew the barrel.
    Smashed {
        cx: u16,
        cz: u16,
        qx: i32,
        qy: i32,
        qz: i32,
    },
}

/// One inventory slot. Empty ⇔ `count == 0`; emptied slots zero **all
/// three** fields so the state hash stays canonical.
///
/// `cond` is the stack's condition in **hundredths of a point** (item
/// durability v0, DECISIONS.md 2026-08-15): 10 000 is the reference's
/// 100-point stone hatchet, and the wear per landed hit is
/// `NodeDef::wear_for`'s (tool, node) row. Zero means either "this item
/// carries no condition" (`GatherContent::cond_max_of` = 0 — wood, stone,
/// every non-tool) or "this tool is dead" — the two are told apart by the
/// content table, never by the stack (`gather::swing`'s Q4 guard). Last
/// field on purpose: `item` and `count` keep their bit positions, so every
/// codec that moved for this moved loudly (wall 6) and none silently.
///
/// A field on the stack rather than a side table, and the reason is the
/// failure mode: a missed site here is a **build error**, where a missed
/// site under a side table is a wrong condition with the goldens, replay
/// and clippy all green (the section-open row's three-green-gates shape).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ItemStack {
    pub item: u16,
    pub count: u16,
    pub cond: u16,
}

/// Add `amount` of `item` to an inventory: top up matching stacks in slot
/// order, then fill empty slots. Returns what actually fit.
///
/// **What does not fit is destroyed here, and that is now the exception
/// rather than the rule** — `inv_add_spilling` is the payout adder, and it
/// hands the remainder to a caller-owned spill buffer that `world.rs`
/// stands up as a bag at the player's feet. This bare form stays for the
/// paths that genuinely have nowhere to put a leftover (a container write,
/// an admin give) and for `inv_add_spilling`'s own two calls.
///
/// A `stack_max` of zero adds nothing and **writes nothing**. Without that
/// guard the empty-slot pass sets `s.item = item` before computing a take
/// of zero, so an item with no stack rule stamped its index across every
/// empty slot as `{ item, count: 0 }` — non-canonical empties, which
/// `world.rs`'s hash reads unconditionally, so two shards that had handled
/// a zero-ceiling item differently would diverge on state a player cannot
/// see. Three of the nine callers guarded it by hand; the other six did
/// not, and `deploy.rs`'s pick-up already documented hitting it.
///
/// `cond` is stamped on **fresh** stacks only (the empty-slot pass) — the
/// condition a mint of this item is born with. A payout, a craft and a
/// loot roll pass `GatherContent::cond_max_of(item)` so a new tool arrives
/// whole (Q3: re-craft IS the repair, so a crafted tool at 0 would repair
/// nothing); a path moving an *existing* stack passes that stack's own
/// `cond`, or looting would mend it. Top-ups never touch it: an item that
/// stacks past 1 carries no condition (content rule V7), so there are
/// never two conditions to reconcile.
pub fn inv_add(
    inv: &mut [ItemStack; INV_SLOTS],
    item: u16,
    amount: u16,
    stack_max: u16,
    cond: u16,
) -> u16 {
    if stack_max == 0 {
        return 0;
    }
    let mut left = amount;
    for s in inv.iter_mut() {
        if left == 0 {
            return amount;
        }
        if s.count > 0 && s.item == item && s.count < stack_max {
            let take = (stack_max - s.count).min(left);
            s.count += take;
            left -= take;
        }
    }
    for s in inv.iter_mut() {
        if left == 0 {
            return amount;
        }
        if s.count == 0 {
            s.item = item;
            let take = stack_max.min(left);
            s.count = take;
            s.cond = cond;
            left -= take;
        }
    }
    amount - left
}

/// Pay `amount` of `item` into an inventory and put whatever will not fit
/// into `spill` instead of destroying it. Returns what reached the
/// inventory — **not** what was paid, because that return feeds
/// `EV_GATHER`/`EV_CRAFT_DONE`, whose meaning is "this entered your hands"
/// (`backpack.rs`'s loot path pays in the same currency) and a spilled
/// stack did not.
///
/// The spill is an ordinary `[ItemStack; INV_SLOTS]` the caller owns for
/// the tick, which is what lets `gather` and `craft` keep their promise not
/// to own a container store: they name what fell, `world.rs` decides where
/// it lands, exactly as `Swing::Smashed` already splits the barrel's bit
/// from the barrel's loot. A spill that overflows its own 30 slots loses
/// the excess — bounded and stated, though a single tick's payout is at
/// most two item kinds and cannot reach it.
pub fn inv_add_spilling(
    inv: &mut [ItemStack; INV_SLOTS],
    spill: &mut [ItemStack; INV_SLOTS],
    item: u16,
    amount: u16,
    stack_max: u16,
    cond: u16,
) -> u16 {
    let added = inv_add(inv, item, amount, stack_max, cond);
    if added < amount {
        inv_add(spill, item, amount - added, stack_max, cond);
    }
    added
}

/// One slot's life record. `respawn_at == 0` ⇒ standing (damaged);
/// nonzero ⇒ harvested until that tick. Absent from the store ⇒ pristine.
#[derive(Clone, Copy, Debug, Default)]
pub struct SlotLife {
    pub cx: u16,
    pub cz: u16,
    pub hits: u16,
    pub respawn_at: u64,
}

/// The server's "one bit + one timer per slot" (TERRAIN.md §2), stored
/// sparsely: only touched slots occupy an entry. Capacity exceeds the
/// ~8–12 k live slots a seed produces (TERRAIN.md §6), so harvested
/// entries always fit; overflow can only involve standing-damage records,
/// which evict lowest-hits-first (the evicted node heals to pristine —
/// bounded memory priced as forgiveness, never unbounded growth).
pub struct SlotLives {
    entries: [SlotLife; MAX_SLOT_LIVES],
    len: usize,
}

impl SlotLives {
    pub fn new() -> Self {
        Self {
            entries: [SlotLife::default(); MAX_SLOT_LIVES],
            len: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn entries(&self) -> &[SlotLife] {
        &self.entries[..self.len]
    }

    /// Replace the store from a decoded world save. Boot-only
    /// (`worldsave.rs`).
    pub(crate) fn restore(&mut self, recs: &[SlotLife]) {
        self.len = recs.len().min(MAX_SLOT_LIVES);
        self.entries[..self.len].copy_from_slice(&recs[..self.len]);
    }

    pub fn find(&self, cx: u16, cz: u16) -> Option<&SlotLife> {
        self.entries[..self.len]
            .iter()
            .find(|e| e.cx == cx && e.cz == cz)
    }

    fn index_of(&self, cx: u16, cz: u16) -> Option<usize> {
        self.entries[..self.len]
            .iter()
            .position(|e| e.cx == cx && e.cz == cz)
    }

    /// True while the slot is inside its harvested window.
    pub fn is_harvested(&self, cx: u16, cz: u16) -> bool {
        self.find(cx, cz).is_some_and(|e| e.respawn_at != 0)
    }

    /// Entry for `(cx, cz)`, inserting a fresh one if needed. At capacity
    /// the lowest-hits standing entry is evicted; None only when every
    /// entry is harvested (arithmetically unreachable: capacity exceeds
    /// the island's slot count), which refuses the hit.
    fn find_or_insert(&mut self, cx: u16, cz: u16) -> Option<&mut SlotLife> {
        if let Some(i) = self.index_of(cx, cz) {
            return Some(&mut self.entries[i]);
        }
        let at = if self.len < MAX_SLOT_LIVES {
            let i = self.len;
            self.len += 1;
            i
        } else {
            let mut best: Option<usize> = None;
            for (i, e) in self.entries.iter().enumerate() {
                if e.respawn_at == 0 && best.is_none_or(|b| e.hits < self.entries[b].hits) {
                    best = Some(i);
                }
            }
            best?
        };
        self.entries[at] = SlotLife {
            cx,
            cz,
            hits: 0,
            respawn_at: 0,
        };
        Some(&mut self.entries[at])
    }

    /// Release every entry whose respawn tick has arrived, reporting each
    /// via `events` (EV_SLOT_RESPAWNED). Swap-remove keeps the store
    /// dense; the order it produces is deterministic like everything else.
    pub fn respawn_due(&mut self, tick: u64, events: &mut EventQueue) {
        let mut i = 0;
        while i < self.len {
            let e = self.entries[i];
            if e.respawn_at != 0 && tick >= e.respawn_at {
                events.push(crate::world::EV_SLOT_RESPAWNED, cell_key(e.cx, e.cz), 0, 0);
                self.len -= 1;
                self.entries[i] = self.entries[self.len];
            } else {
                i += 1;
            }
        }
    }
}

impl Default for SlotLives {
    fn default() -> Self {
        Self::new()
    }
}

/// Cell coords packed for event args.
#[inline]
pub fn cell_key(cx: u16, cz: u16) -> u32 {
    ((cx as u32) << 16) | cz as u32
}

/// The weak-spot mark after `n` landed hits by `pid` on the node at
/// `(cx, cz)`: a heading over the 256-entry yaw LUT, pointing from the
/// node toward where the swinger must stand. Per-player (the reference
/// mechanic's mark is yours alone) and pure — server, replay, and any
/// future client-side ghost all derive the same mark.
#[inline]
pub fn weak_mark8(seed: u64, cx: u16, cz: u16, pid: u32, n: u16) -> u8 {
    let h = cell_hash(seed, cx as i32, cz as i32, CH_WEAK);
    (splitmix64(h ^ ((pid as u64) << 16) ^ n as u64) >> 32) as u8
}

/// One player's swing gate + target pick + payout. Called every tick for
/// every active player, after movement — bounded: 3×3 scatter cells
/// scanned only on a swing tick, and read through the same memo the
/// collision path uses (`cache`).
///
/// **That memo is not an optimisation here, it is the difference between a
/// smooth tick and a spike**, and this function was the one caller in the
/// tick that went around it. `occupy.rs` says why in its own words — a
/// `terrain::scatter` is ~60 `noise2` evaluations, so a 3×3 ring resolved
/// cold is ~540 — and it says a movement step must never re-derive slots
/// that way. A swing is not a movement step, but it reads the *same nine
/// cells at the same position on the same tick*, so the lines it wants are
/// the ones `Occupants::blocks` just filled. Measured 2026-08-11 with
/// `server/bin/profile.rs` at `MAX_PLAYERS`: a hundred bodies whose swing
/// cooldowns had lined up cost 1.9 ms in one `World::tick` against a 33 ms
/// budget — an 8× spike over the same shard's average — and cold scatter
/// was all of it. `SWING_INTERVAL_TICKS` makes that alignment a normal
/// event, not a contrivance: everyone who spawns together swings together.
///
/// Exact, not approximate, for `SlotCache`'s stated reason: scatter is a
/// pure function of `(seed, cell)`, so a hit and a miss return the same
/// bits and eviction can only change how long an answer took. The cache is
/// not sim state and is not hashed.
///
/// Returns `Swing::Free` when a swing was taken and nothing absorbed it —
/// the cadence is paid and the arm is still moving, so the caller hands it
/// to `combat::strike`. `Absorbed` means either no swing this tick (button
/// up, or still on cooldown) or a node took the hit: one arm, one target,
/// and the nearest standing thing is always the nearer claim on it.
/// `Smashed` is a barrel that came apart — absorbed, and with a container
/// owed at the address it names.
///
/// `spill` catches yield the swinger's inventory could not hold. It is the
/// caller's buffer for the tick and this function only writes into it; the
/// caller stands it up as a bag, because gather owns the slot bit and not
/// the container store — the same split `Smashed` already makes.
#[allow(clippy::too_many_arguments)]
pub fn swing(
    seed: u64,
    tick: u64,
    gc: &GatherContent,
    lc: &LootContent,
    scatter: &ScatterTable,
    haven: &terrain::Haven,
    cache: &mut crate::occupy::SlotCache,
    lives: &mut SlotLives,
    events: &mut EventQueue,
    p: &mut Player,
    spill: &mut [ItemStack; INV_SLOTS],
) -> Swing {
    if p.frame.buttons & BTN_PRIMARY == 0 || tick < p.next_swing {
        return Swing::Absorbed;
    }
    p.next_swing = tick + SWING_INTERVAL_TICKS;

    // **The arm moved, and that is a fact about a body other people are
    // drawing.** Pushed HERE and nowhere else, because the two lines above
    // are the cadence gate: this is the only point in the tree that runs
    // exactly once per swing regardless of what the swing goes on to find.
    // Every exit below it — a whiff, a refusal, a free arm handed to flesh,
    // a smashed barrel — is downstream of a decision the swinger has
    // already committed to, and a fact that fires only when something was
    // hit is a HIT fact, not a swing fact. This lane already has one of
    // those, and `EV_HIT` is unicast to the attacker for exactly that
    // reason. `NOW.md` §0sw: the commonest swing in the game is the one
    // that misses, and it drew nothing on any screen but the swinger's.
    events.push(crate::world::EV_SWING, p.id, 0, 0);

    let px = p.body.qx as f32 * POS_XZ_Q;
    let py = p.body.qy as f32 * POS_Y_Q;
    let pz = p.body.qz as f32 * POS_XZ_Q;
    let (fx, fz) = yaw_dir(p.frame.yaw);
    let pcx = crate::fmath::floor_i32(px / CELL_SIZE);
    let pcz = crate::fmath::floor_i32(pz / CELL_SIZE);

    // Nearest standing swingable slot in reach, inside the aim cone.
    let mut best: Option<Target> = None;
    let mut dz_cell = -1;
    while dz_cell <= 1 {
        let mut dx_cell = -1;
        while dx_cell <= 1 {
            let cx = pcx + dx_cell;
            let cz = pcz + dz_cell;
            let s = cache.slot(seed, scatter, haven, cx, cz);
            if let Some(ni) = target_index(s.occupant) {
                let dx = s.x - px;
                let dy = s.y - py;
                let dz = s.z - pz;
                let d2 = dx * dx + dz * dz;
                let aimed = d2 <= POINT_BLANK_M2 || {
                    let dot = dx * fx + dz * fz;
                    dot > CONE_COS * d2.sqrt()
                };
                if d2 <= REACH_M * REACH_M
                    && crate::fmath::fabs(dy) <= DY_MAX_M
                    && aimed
                    && best.as_ref().is_none_or(|b| d2 < b.d2)
                    && !lives.is_harvested(cx as u16, cz as u16)
                {
                    let (or_m, otop_m) = terrain::occupant_volume(s.occupant);
                    best = Some(Target {
                        d2,
                        ox: -dx,
                        oz: -dz,
                        cx: cx as u16,
                        cz: cz as u16,
                        ni,
                        pos: (s.x, s.y, s.z),
                        r: or_m * s.scale,
                        top: otop_m * s.scale,
                    });
                }
            }
            dx_cell += 1;
        }
        dz_cell += 1;
    }
    let Some(Target {
        d2,
        ox,
        oz,
        cx,
        cz,
        ni,
        pos,
        r: hit_r,
        top: hit_top,
    }) = best
    else {
        return Swing::Free; // whiff — the cooldown is paid, the arm is free
    };

    if ni == BARREL_TARGET {
        return smash(lc, seed, tick, cx, cz, pos, lives, events, p);
    }

    let def = &gc.nodes[ni];
    if def.output == NO_ITEM || def.output as usize >= MAX_ITEM_DEFS {
        return Swing::Free; // inert content (or a table the bake would have refused)
    }
    // What is in hand decides whether this node answers at all, so it is
    // read here rather than beside the payout below.
    //
    // **A dead tool reads as no tool** (Q4, operator 2026-08-15): a stack
    // at condition zero whose item declares a ceiling stays in the hand
    // and stops being a tool, so `yield_for` falls back to the hand row —
    // which on every swung node is 0 since the 2026-08-15 hand-row
    // deletion, so the swing lands in the refusal below and never on the
    // wall behind the node (`Swing::Refused`). The ceiling check is what
    // keeps every non-tool honest: wood is `cond == 0` forever and is
    // still wood in the hand.
    let raw_held = if p.inv[p.frame.sel as usize].count > 0 {
        p.inv[p.frame.sel as usize].item
    } else {
        NO_ITEM
    };
    let held_dead = raw_held != NO_ITEM
        && p.inv[p.frame.sel as usize].cond == 0
        && gc.cond_max_of(raw_held) > 0;
    let held = if held_dead { NO_ITEM } else { raw_held };
    // **A tool this node pays nothing for does not get to destroy it.**
    // Content with no `hand` row bakes `hand_yield: 0` and `yield_for`
    // falls back to it for anything not in the tool table, so from
    // 2026-08-15 a bare fist — or a torch, or a hammer — reads 0 on every
    // swung node (content/gatherables.toml; operator: *"you cant smash
    // trees with ur hans lol u need a rock"*).
    //
    // The refusal has to be HERE, above `find_or_insert`, because the
    // budget spend below is unconditional: without it ten bare-hand swings
    // would exhaust a tree, pay nothing, and put it on a 20–45 min
    // respawn — a griefing hole, and a self-grief hole for anyone who lost
    // their rock. It also keeps the swing out of `SlotLives`, which is a
    // bounded store a free verb must not be able to fill.
    //
    // The refusal is announced (wire v42; it was silent from 2026-08-15
    // to this bump, the dead-button shape `NOW.md` §0eat is about).
    // Re-using `EV_GATHER` with `added = 0` was NOT the cheap way out:
    // that encoding is already spoken for as the spill signal (see the
    // payout comment below), so it would have made one wire fact mean two
    // things. Bounded by the swing cadence — one refusal per
    // `SWING_INTERVAL_TICKS`, never one per tick.
    if def.yield_for(held) == 0 {
        let why = if held_dead {
            REFUSE_G_BROKEN
        } else {
            REFUSE_G_TOOL
        };
        events.push(EV_GATHER_REFUSED, p.id, ((raw_held as u32) << 16) | why, 0);
        // Refused, not Free: the arm carries on to flesh and never to
        // structure — `Swing::Refused` says why in full.
        return Swing::Refused;
    }
    let Some(life) = lives.find_or_insert(cx, cz) else {
        return Swing::Free; // store exhausted by harvested entries — refuse the hit
    };
    // The weak-spot chase: switching nodes restarts it; the mark only
    // exists after the first landed hit. A hit landed while standing in
    // the current mark's sector spends the content's extra budget;
    // point-blank has no bearing to judge, so it never marks.
    let ck = cell_key(cx, cz);
    if p.ws_cell != ck {
        p.ws_cell = ck;
        p.ws_hits = 0;
    }
    let mut weak_hit = false;
    if def.weak_pct > 0 && p.ws_hits > 0 && d2 > POINT_BLANK_M2 {
        let mark = weak_mark8(seed, cx, cz, p.id, p.ws_hits);
        let (wx, wz) = yaw_dir((mark as u16) << 8);
        weak_hit = ox * wx + oz * wz > WEAK_COS * d2.sqrt();
    }
    p.ws_hits = p.ws_hits.saturating_add(1);

    // Spend the swing's budget against the node's pool. A marked swing
    // takes a bigger bite (`weak_pct`) and is paid pro rata for it, so
    // the node's total never moves and only the swing COUNT falls —
    // `NodeDef::weak_pct` has the reasoning. The last swing takes
    // whatever is left rather than overdrawing, which is what keeps the
    // total exact instead of approximately right.
    let budget = def.hits as u32 * HIT_UNIT;
    let want = if weak_hit {
        HIT_UNIT + def.weak_pct as u32
    } else {
        HIT_UNIT
    };
    let take = want.min(budget - life.hits as u32);
    life.hits += take as u16;
    let exhausted = life.hits as u32 >= budget;
    if exhausted {
        let jitter = splitmix64(cell_hash(seed, cx as i32, cz as i32, CH_RESPAWN) ^ tick);
        life.respawn_at = tick + RESPAWN_MIN_TICKS + jitter % RESPAWN_RANGE_TICKS;
    }

    // **The swing has landed, so it leaves the mark an arrow leaves.**
    // Reached only past the tool refusal and the store insert above, which
    // is what makes it mean "this swing bit the node" rather than "a button
    // was down" — a refused swing scuffs nothing, and `Swing::Refused`
    // already returns before here.
    //
    // `EV_IMPACT` is reused rather than joined by a second event, and that
    // is the whole slice: the fact is *a surface was struck at this point*,
    // which is neither an arrow's fact nor a swing's. It is already
    // broadcast, already carries a quantized point and a surface class, and
    // `render/decal.rs` is already its single reader — so a mark on a tree
    // costs no wire byte, no `PROTO_VER` bump and no client line
    // (`NOW.md` §0mk item 1).
    if let Some((mx, my, mz)) = skin_point(pos, ox, oz, d2, hit_r, hit_top, py + EYE_M) {
        let qx = crate::fmath::floor_i32(mx / POS_XZ_Q);
        let qy = crate::fmath::floor_i32(my / POS_Y_Q);
        let qz = crate::fmath::floor_i32(mz / POS_XZ_Q);
        events.push(
            crate::world::EV_IMPACT,
            (crate::ranged::SURF_WORLD as u32) << 24 | qx as u32,
            qz as u32,
            qy as u32,
        );
    }

    // Pay pro rata for the budget spent, less the share this node holds
    // back for whoever finishes it.
    //
    // **Exact by construction, and it has to be.** Paying
    // `floor(per_swing_share)` each swing loses the remainder every time
    // and a node quietly pays less than `hits × per-hit` — the first cut
    // of this lost 3 of 30 on the fixture's stone. So the running total
    // is the difference of two CUMULATIVE floors (drift can never
    // exceed one unit and always closes by the last swing), and the
    // finisher's share is the exact remainder `total − pool` rather than
    // a second independent percentage. The two therefore sum to `total`
    // for any content, divisible by 100 or not.
    //
    // Both halves read the tool in hand on THIS swing, so switching to a
    // better tool for the last hit pays a better finish — the
    // reference's shape too (their HQM comes only off the final strike).
    // A switch mid-node re-bases the schedule, so the cumulative
    // difference is saturating: a worse tool never claws yield back.
    let full = def.yield_for(held) as u64;
    let total = full * def.hits as u64;
    let pool = total * (100 - def.finish_pct as u64) / 100;
    let spent_after = life.hits as u64;
    let spent_before = spent_after - take as u64;
    let budget = budget as u64;
    let mut pay = (pool * spent_after / budget).saturating_sub(pool * spent_before / budget);
    if exhausted {
        pay += total - pool;
    }
    let pay = pay.min(u16::MAX as u64) as u16;
    // A full pack no longer eats the swing: what will not fit goes to the
    // spill and `world.rs` drops it where the swinger stands. A fresh
    // stack of the output is minted at the item's own condition ceiling —
    // 0 for every resource, and whole for the fixture items that double
    // as tools (`inv_add`'s `cond` doc).
    let added = inv_add_spilling(
        &mut p.inv,
        spill,
        def.output,
        pay,
        gc.stack_max[def.output as usize],
        gc.cond_max[def.output as usize],
    );
    // **`pay > 0` is what makes `added == 0` mean something.** The cumulative
    // schedule above legitimately pays nothing on some swings — `pool` need
    // not divide `budget`, so a node worth 10 over 20 hits pays on half of
    // them — and an `EV_GATHER` for one of those said "0 units entered your
    // inventory" from a swing that was never owed any. That is the same
    // sentence a FULL PACK produces, so the client could not tell the two
    // apart and correctly showed neither (`client-core`'s `if added > 0`).
    //
    // A swing that paid nothing is not a gather, so it no longer announces
    // itself, and `added == 0` on a surviving `EV_GATHER` now says exactly
    // one thing: it was paid and none of it fit. That is the spill signal,
    // carried on a field the wire already has — see `world.rs`'s doc line.
    // The loot path never needed this (`backpack.rs` skips a `took == 0`
    // slot) and the secondary payout below is already guarded by `sec_pay`.
    if pay > 0 {
        events.push(
            EV_GATHER,
            p.id,
            ((def.output as u32) << 16) | added as u32,
            0,
        );
    }
    // The side payout: its own `EV_GATHER`, so the client's toast stack
    // reads `+10 Cloth` *and* `+5 Berries` rather than one line that lies
    // about half of what arrived. Two pushes on a bounded ring, once per
    // landed swing, on the nodes whose content declares one.
    let (sec_item, sec_pay) = def.secondary;
    if sec_item != NO_ITEM && (sec_item as usize) < MAX_ITEM_DEFS && sec_pay > 0 {
        let got = inv_add_spilling(
            &mut p.inv,
            spill,
            sec_item,
            sec_pay,
            gc.stack_max[sec_item as usize],
            gc.cond_max[sec_item as usize],
        );
        events.push(EV_GATHER, p.id, ((sec_item as u32) << 16) | got as u32, 0);
    }
    // **Wear, after the payout, on a landed node hit only** — never on a
    // whiff, a refusal or a smash, because a whiff that wore would put a
    // `SUB_INV` message on the wire on every swing of every player
    // forever. By the node's own rate for this tool (`wear_for` — the
    // (tool, node) table is the wrong-tool predicate), `saturating_sub`
    // so the last point is spent and never owed. The held slot cannot
    // have changed since the read above: nothing between there and here
    // touches `p.inv[p.frame.sel]`'s identity — `inv_add_spilling` tops
    // up and fills, and V7 (condition ⇒ stack of 1) is what guarantees a
    // payout can never merge INTO the held tool's slot.
    let wear = def.wear_for(held);
    if wear > 0 {
        let s = &mut p.inv[p.frame.sel as usize];
        s.cond = s.cond.saturating_sub(wear);
    }
    if exhausted {
        events.push(EV_SLOT_HARVESTED, ck, occupant_of(ni), 0);
        p.ws_cell = NO_CELL;
        p.ws_hits = 0;
        return Swing::Absorbed;
    }
    if def.weak_pct > 0 {
        let next = weak_mark8(seed, cx, cz, p.id, p.ws_hits);
        events.push(
            EV_WEAK_MARK,
            p.id,
            ck,
            ((weak_hit as u32) << 8) | next as u32,
        );
    }
    Swing::Absorbed // the node took it
}

/// A swing that landed on a barrel slot.
///
/// A barrel is not a node and this is not `NodeDef` with the fields blanked
/// out: no tool row, no hand yield, no weak spot, no payout into the
/// swinger's hands. What it shares with a node is the *bit* — the same
/// `SlotLives` entry, the same jittered respawn window (DECISIONS.md §open
/// "node/barrel respawn 20–45 min" names both), and the same
/// `EV_SLOT_HARVESTED` on the way out, so a client that already hides a
/// felled tree hides a smashed barrel with no wire change at all.
#[allow(clippy::too_many_arguments)]
fn smash(
    lc: &LootContent,
    seed: u64,
    tick: u64,
    cx: u16,
    cz: u16,
    spos: (f32, f32, f32),
    lives: &mut SlotLives,
    events: &mut EventQueue,
    p: &mut Player,
) -> Swing {
    let hits = lc.hits(LOOT_BARREL);
    if hits == 0 {
        return Swing::Free; // inert loot content: nothing here to break
    }
    // A barrel has no glint to chase, so aiming at one ends any chase in
    // progress rather than leaving the mark pointed at a cell that pays no
    // bonus.
    p.ws_cell = NO_CELL;
    p.ws_hits = 0;

    let Some(life) = lives.find_or_insert(cx, cz) else {
        return Swing::Free; // store exhausted by harvested entries
    };
    life.hits += 1;
    if life.hits < hits {
        return Swing::Absorbed;
    }
    let jitter = splitmix64(cell_hash(seed, cx as i32, cz as i32, CH_RESPAWN) ^ tick);
    life.respawn_at = tick + RESPAWN_MIN_TICKS + jitter % RESPAWN_RANGE_TICKS;
    events.push(
        EV_SLOT_HARVESTED,
        cell_key(cx, cz),
        occupant_of(BARREL_TARGET),
        0,
    );
    Swing::Smashed {
        cx,
        cz,
        qx: quant_xz(spos.0),
        qy: quant_y(spos.1),
        qz: quant_xz(spos.2),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inv_add_stacks_then_fills_then_loses() {
        let mut inv = [ItemStack::default(); INV_SLOTS];
        assert_eq!(inv_add(&mut inv, 3, 70, 100, 0), 70);
        assert_eq!(inv_add(&mut inv, 3, 70, 100, 0), 70);
        assert_eq!(
            inv[0],
            ItemStack {
                item: 3,
                count: 100,
                cond: 0,
            }
        );
        assert_eq!(
            inv[1],
            ItemStack {
                item: 3,
                count: 40,
                cond: 0
            }
        );
        // Fill every slot, then overflow is lost.
        for s in inv.iter_mut() {
            *s = ItemStack {
                item: 3,
                count: 100,
                cond: 0,
            };
        }
        assert_eq!(inv_add(&mut inv, 3, 50, 100, 0), 0);
        // A different item can't ride an existing stack.
        inv[4] = ItemStack::default();
        assert_eq!(inv_add(&mut inv, 7, 250, 100, 0), 100);
        assert_eq!(
            inv[4],
            ItemStack {
                item: 7,
                count: 100,
                cond: 0,
            }
        );
    }

    /// A stack ceiling of zero must add nothing AND write nothing. Before
    /// the guard, the empty-slot pass set `s.item` before computing a take
    /// of zero, so this stamped `{ item: 9, count: 0 }` across all 30
    /// slots — non-canonical empties, which `world.rs`'s state hash reads
    /// unconditionally. Red without the guard: the second assert fails on
    /// `inv[0].item == 9`.
    #[test]
    fn a_ceiling_of_zero_writes_nothing() {
        let mut inv = [ItemStack::default(); INV_SLOTS];
        assert_eq!(
            inv_add(&mut inv, 9, 5, 0, 0),
            0,
            "nothing fits at ceiling 0"
        );
        assert!(
            inv.iter().all(|s| *s == ItemStack::default()),
            "a zero ceiling must leave the inventory canonically empty"
        );
    }

    /// The payout adder hands the remainder to the spill instead of
    /// destroying it, and reports only what reached the hands.
    #[test]
    fn spilling_keeps_what_the_pack_could_not_hold() {
        let mut inv = [ItemStack::default(); INV_SLOTS];
        let mut spill = [ItemStack::default(); INV_SLOTS];
        // One free slot, ceiling 100, paid 250: 100 lands, 150 spills.
        for s in inv.iter_mut() {
            *s = ItemStack {
                item: 3,
                count: 100,
                cond: 0,
            };
        }
        inv[7] = ItemStack::default();
        assert_eq!(inv_add_spilling(&mut inv, &mut spill, 7, 250, 100, 0), 100);
        assert_eq!(crate::craft::inv_count(&spill, 7), 150, "the rest fell");
        // Nothing spills when it all fits, and the spill is untouched.
        let mut inv2 = [ItemStack::default(); INV_SLOTS];
        let mut spill2 = [ItemStack::default(); INV_SLOTS];
        assert_eq!(inv_add_spilling(&mut inv2, &mut spill2, 4, 30, 100, 0), 30);
        assert!(spill2.iter().all(|s| s.count == 0));
    }

    #[test]
    fn yield_for_falls_back_to_hand() {
        let mut def = NodeDef::INERT;
        def.hand_yield = 5;
        def.tools[0] = (2, 20);
        assert_eq!(def.yield_for(2), 20);
        assert_eq!(def.yield_for(9), 5);
        assert_eq!(def.yield_for(NO_ITEM), 5);
    }

    #[test]
    fn slot_lives_evicts_lowest_hits_standing_only() {
        let mut lives = SlotLives::new();
        // Fill to capacity: one harvested, the rest standing with rising hits.
        for i in 0..MAX_SLOT_LIVES {
            let e = lives.find_or_insert(i as u16, 0).unwrap();
            e.hits = i as u16 + 2;
            if i == 0 {
                e.respawn_at = 999; // harvested — never evicted
            }
        }
        assert_eq!(lives.len(), MAX_SLOT_LIVES);
        // Insert past capacity: entry (1,0) has the lowest standing hits.
        let e = lives.find_or_insert(9999, 9999).unwrap();
        assert_eq!((e.cx, e.cz, e.hits), (9999, 9999, 0));
        assert_eq!(lives.len(), MAX_SLOT_LIVES);
        assert!(lives.find(1, 0).is_none(), "standing lowest-hits evicted");
        assert!(lives.find(0, 0).is_some(), "harvested survives eviction");
    }

    #[test]
    fn respawn_due_releases_and_reports() {
        let mut lives = SlotLives::new();
        lives.find_or_insert(5, 6).unwrap().respawn_at = 100;
        lives.find_or_insert(7, 8).unwrap().respawn_at = 200;
        lives.find_or_insert(9, 9).unwrap().hits = 3; // standing: untouched
        let mut ev = EventQueue::default();
        lives.respawn_due(150, &mut ev);
        assert_eq!(lives.len(), 2);
        assert!(lives.find(5, 6).is_none());
        assert!(lives.find(7, 8).is_some());
        assert!(lives.find(9, 9).is_some());
        assert_eq!(ev.len(), 1);
        assert_eq!(ev.entries()[0].a, cell_key(5, 6));
    }
}
