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
use crate::world::{EventQueue, Player, EV_GATHER, EV_SLOT_HARVESTED, EV_WEAK_MARK};
use crate::yaw_lut::yaw_dir;

/// Sentinel: no item. Doubles as the bare-hand "held item".
pub const NO_ITEM: u16 = u16::MAX;

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
}

/// The whole gather ruleset the sim knows: per-archetype node rules plus
/// per-item stack ceilings. Construction input like the seed — the WAL
/// pins the content hash it was baked from (CONTENT.md §0).
#[derive(Clone, Copy, Debug)]
pub struct GatherContent {
    /// Indexed by `Occupant as usize - 1` (Tree..Bush).
    pub nodes: [NodeDef; GATHERABLE_KINDS],
    pub stack_max: [u16; MAX_ITEM_DEFS],
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

    /// Inert: nothing is gatherable. `World::new` starts here; the boot
    /// path installs the baked table before the first tick.
    pub const EMPTY: Self = Self {
        nodes: [NodeDef::INERT; GATHERABLE_KINDS],
        stack_max: [0; MAX_ITEM_DEFS],
        item_count: 0,
    };

    /// Synthetic table for the parity/replay/alloc gates. Deliberately
    /// unlike game content: outputs double as tools (item 0 gathers item
    /// 1 faster and vice versa) so bot runs cover the tool-yield path the
    /// moment a bot's slot 0 fills. Real values bake from content/*.toml.
    pub fn probe_fixture() -> Self {
        let mut c = Self::EMPTY;
        // Nine, not eight: every index below is spoken for by a fixture
        // deployable's `item` (hearth 2, workbench 3, door 4, bag 5, fire
        // 6, lock 7) and the recycler is the ninth thing to need one. Two
        // deployables sharing an item is what `DeployContent::
        // probe_fixture` warns about at the lock row — the give-back hands
        // out the wrong object — so the fixture widens rather than
        // doubling up.
        c.item_count = 9;
        let mut i = 0;
        while i < 9 {
            c.stack_max[i] = 100;
            i += 1;
        }
        // (output, hits, hand, weak %, finish %, tool-item, tool-yield).
        // Finish shares are deliberately varied and deliberately NOT the
        // shipped content's — a fixture that matches the game hides a
        // bake that ignores the column.
        let rows: [(u16, u16, u16, u16, u16, u16, u16); GATHERABLE_KINDS] = [
            // Tree and Bush withhold nothing, so the mark and side-payout
            // gates read pay directly. Stone and Sulfur carry the finish
            // coverage, at different shares and deliberately NOT the
            // shipped content's — a fixture that matches the game hides a
            // bake that ignores the column.
            (0, 4, 7, 100, 0, 1, 13),     // Tree
            (1, 5, 6, 50, 40, 0, 11),     // StoneNode
            (2, 6, 3, 25, 0, 0, 9),       // MetalNode: pays evenly
            (3, 6, 3, 75, 10, 1, 9),      // SulfurNode
            (4, 1, 10, 0, 0, NO_ITEM, 0), // Bush: one-hit pickup, no mark
        ];
        let mut k = 0;
        while k < GATHERABLE_KINDS {
            let (out, hits, hand, weak, finish, tool, per) = rows[k];
            c.nodes[k] = NodeDef {
                output: out,
                hits,
                hand_yield: hand,
                weak_pct: weak,
                finish_pct: finish,
                tools: [(NO_ITEM, 0); MAX_TOOLS_PER_NODE],
                secondary: (NO_ITEM, 0),
            };
            if tool != NO_ITEM {
                c.nodes[k].tools[0] = (tool, per);
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

/// One inventory slot. Empty ⇔ `count == 0`; emptied slots zero both
/// fields so the state hash stays canonical.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ItemStack {
    pub item: u16,
    pub count: u16,
}

/// Add `amount` of `item` to an inventory: top up matching stacks in slot
/// order, then fill empty slots. Returns what actually fit — the rest is
/// lost (documented policy, DECISIONS.md §open: ground drops land with
/// the death/backpack slice).
pub fn inv_add(inv: &mut [ItemStack; INV_SLOTS], item: u16, amount: u16, stack_max: u16) -> u16 {
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
            left -= take;
        }
    }
    amount - left
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
/// scanned only on a swing tick.
///
/// Returns `Swing::Free` when a swing was taken and nothing absorbed it —
/// the cadence is paid and the arm is still moving, so the caller hands it
/// to `combat::strike`. `Absorbed` means either no swing this tick (button
/// up, or still on cooldown) or a node took the hit: one arm, one target,
/// and the nearest standing thing is always the nearer claim on it.
/// `Smashed` is a barrel that came apart — absorbed, and with a container
/// owed at the address it names.
#[allow(clippy::too_many_arguments)]
pub fn swing(
    seed: u64,
    tick: u64,
    gc: &GatherContent,
    lc: &LootContent,
    scatter: &ScatterTable,
    haven: &terrain::Haven,
    lives: &mut SlotLives,
    events: &mut EventQueue,
    p: &mut Player,
) -> Swing {
    if p.frame.buttons & BTN_PRIMARY == 0 || tick < p.next_swing {
        return Swing::Absorbed;
    }
    p.next_swing = tick + SWING_INTERVAL_TICKS;

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
            let s = terrain::scatter(seed, scatter, haven, cx, cz);
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
                    best = Some(Target {
                        d2,
                        ox: -dx,
                        oz: -dz,
                        cx: cx as u16,
                        cz: cz as u16,
                        ni,
                        pos: (s.x, s.y, s.z),
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

    let held = if p.inv[p.frame.sel as usize].count > 0 {
        p.inv[p.frame.sel as usize].item
    } else {
        NO_ITEM
    };
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
    let added = inv_add(
        &mut p.inv,
        def.output,
        pay,
        gc.stack_max[def.output as usize],
    );
    events.push(
        EV_GATHER,
        p.id,
        ((def.output as u32) << 16) | added as u32,
        0,
    );
    // The side payout: its own `EV_GATHER`, so the client's toast stack
    // reads `+10 Cloth` *and* `+5 Berries` rather than one line that lies
    // about half of what arrived. Two pushes on a bounded ring, once per
    // landed swing, on the nodes whose content declares one.
    let (sec_item, sec_pay) = def.secondary;
    if sec_item != NO_ITEM && (sec_item as usize) < MAX_ITEM_DEFS && sec_pay > 0 {
        let got = inv_add(
            &mut p.inv,
            sec_item,
            sec_pay,
            gc.stack_max[sec_item as usize],
        );
        events.push(EV_GATHER, p.id, ((sec_item as u32) << 16) | got as u32, 0);
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
        assert_eq!(inv_add(&mut inv, 3, 70, 100), 70);
        assert_eq!(inv_add(&mut inv, 3, 70, 100), 70);
        assert_eq!(
            inv[0],
            ItemStack {
                item: 3,
                count: 100
            }
        );
        assert_eq!(inv[1], ItemStack { item: 3, count: 40 });
        // Fill every slot, then overflow is lost.
        for s in inv.iter_mut() {
            *s = ItemStack {
                item: 3,
                count: 100,
            };
        }
        assert_eq!(inv_add(&mut inv, 3, 50, 100), 0);
        // A different item can't ride an existing stack.
        inv[4] = ItemStack::default();
        assert_eq!(inv_add(&mut inv, 7, 250, 100), 100);
        assert_eq!(
            inv[4],
            ItemStack {
                item: 7,
                count: 100
            }
        );
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
