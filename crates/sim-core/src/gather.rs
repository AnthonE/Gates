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
use crate::movement::{POS_XZ_Q, POS_Y_Q};
use crate::rng::{cell_hash, splitmix64};
use crate::terrain::{self, Occupant, ScatterTable, CELL_SIZE};
use crate::world::{EventQueue, Player, EV_GATHER, EV_SLOT_HARVESTED, EV_WEAK_MARK};
use crate::yaw_lut::yaw_dir;

/// Sentinel: no item. Doubles as the bare-hand "held item".
pub const NO_ITEM: u16 = u16::MAX;

/// Sentinel cell key: no weak-spot chase in progress (`Player::ws_cell`).
pub const NO_CELL: u32 = u32::MAX;

/// Occupants that can be gathered: Tree, StoneNode, MetalNode, SulfurNode,
/// Bush — terrain `Occupant` 1..=5. Rock and BarrelSlot are not nodes.
pub const GATHERABLE_KINDS: usize = 5;

/// Tool rows one node archetype can carry (alpha data uses ≤ 4 + hand;
/// bake refuses past this). Structural cap, not a knob.
pub const MAX_TOOLS_PER_NODE: usize = 8;

/// Ticks between swings while the primary button is held: ~47 swings/min
/// at 30 Hz, the melee-band cadence. Paid per swing, hit or whiff.
pub const SWING_INTERVAL_TICKS: u64 = 38;
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
    /// Swings to exhaust the node.
    pub hits: u16,
    /// Units per bare-hand swing.
    pub hand_yield: u16,
    /// Extra yield % on a weak-spot hit (content `weak_spot_bonus_pct`);
    /// 0 disables the mark for this archetype.
    pub weak_pct: u16,
    /// (item index, units per swing) rows; `(NO_ITEM, 0)` = empty row.
    pub tools: [(u16, u16); MAX_TOOLS_PER_NODE],
}

impl NodeDef {
    pub const INERT: Self = Self {
        output: NO_ITEM,
        hits: 0,
        hand_yield: 0,
        weak_pct: 0,
        tools: [(NO_ITEM, 0); MAX_TOOLS_PER_NODE],
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
        c.item_count = 8;
        let mut i = 0;
        while i < 8 {
            c.stack_max[i] = 100;
            i += 1;
        }
        // (output, hits, hand, weak %, tool-item, tool-yield) per archetype.
        let rows: [(u16, u16, u16, u16, u16, u16); GATHERABLE_KINDS] = [
            (0, 4, 7, 100, 1, 13),     // Tree
            (1, 5, 6, 50, 0, 11),      // StoneNode
            (2, 6, 3, 25, 0, 9),       // MetalNode
            (3, 6, 3, 75, 1, 9),       // SulfurNode
            (4, 1, 10, 0, NO_ITEM, 0), // Bush: one-hit pickup, no mark
        ];
        let mut k = 0;
        while k < GATHERABLE_KINDS {
            let (out, hits, hand, weak, tool, per) = rows[k];
            c.nodes[k] = NodeDef {
                output: out,
                hits,
                hand_yield: hand,
                weak_pct: weak,
                tools: [(NO_ITEM, 0); MAX_TOOLS_PER_NODE],
            };
            if tool != NO_ITEM {
                c.nodes[k].tools[0] = (tool, per);
            }
            k += 1;
        }
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
#[allow(clippy::too_many_arguments)]
pub fn swing(
    seed: u64,
    tick: u64,
    gc: &GatherContent,
    scatter: &ScatterTable,
    lives: &mut SlotLives,
    events: &mut EventQueue,
    p: &mut Player,
) {
    if p.frame.buttons & BTN_PRIMARY == 0 || tick < p.next_swing {
        return;
    }
    p.next_swing = tick + SWING_INTERVAL_TICKS;

    let px = p.body.qx as f32 * POS_XZ_Q;
    let py = p.body.qy as f32 * POS_Y_Q;
    let pz = p.body.qz as f32 * POS_XZ_Q;
    let (fx, fz) = yaw_dir(p.frame.yaw);
    let pcx = crate::fmath::floor_i32(px / CELL_SIZE);
    let pcz = crate::fmath::floor_i32(pz / CELL_SIZE);

    // Nearest standing gatherable slot in reach, inside the aim cone.
    // (d2, node→player planar offset, cell, gatherable index.)
    let mut best: Option<(f32, f32, f32, u16, u16, usize)> = None;
    let mut dz_cell = -1;
    while dz_cell <= 1 {
        let mut dx_cell = -1;
        while dx_cell <= 1 {
            let cx = pcx + dx_cell;
            let cz = pcz + dz_cell;
            let s = terrain::scatter(seed, scatter, cx, cz);
            if let Some(ni) = node_index(s.occupant) {
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
                    && best.is_none_or(|(bd2, ..)| d2 < bd2)
                    && !lives.is_harvested(cx as u16, cz as u16)
                {
                    best = Some((d2, -dx, -dz, cx as u16, cz as u16, ni));
                }
            }
            dx_cell += 1;
        }
        dz_cell += 1;
    }
    let Some((d2, ox, oz, cx, cz, ni)) = best else {
        return; // whiff — the cooldown is already paid
    };

    let def = &gc.nodes[ni];
    if def.output == NO_ITEM || def.output as usize >= MAX_ITEM_DEFS {
        return; // inert content (or a table the bake would have refused)
    }
    let Some(life) = lives.find_or_insert(cx, cz) else {
        return; // store exhausted by harvested entries — refuse the hit
    };
    life.hits += 1;
    let exhausted = life.hits >= def.hits;
    if exhausted {
        let jitter = splitmix64(cell_hash(seed, cx as i32, cz as i32, CH_RESPAWN) ^ tick);
        life.respawn_at = tick + RESPAWN_MIN_TICKS + jitter % RESPAWN_RANGE_TICKS;
    }

    // The weak-spot chase: switching nodes restarts it; the mark only
    // exists after the first landed hit. A hit landed while standing in
    // the current mark's sector pays the content bonus; point-blank has
    // no bearing to judge, so it never bonuses.
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

    let held = if p.inv[p.frame.sel as usize].count > 0 {
        p.inv[p.frame.sel as usize].item
    } else {
        NO_ITEM
    };
    let mut pay = def.yield_for(held);
    if weak_hit {
        pay = ((pay as u32 * (100 + def.weak_pct as u32)) / 100).min(u16::MAX as u32) as u16;
    }
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
    if exhausted {
        events.push(EV_SLOT_HARVESTED, ck, ni as u32, 0);
        p.ws_cell = NO_CELL;
        p.ws_hits = 0;
    } else if def.weak_pct > 0 {
        let next = weak_mark8(seed, cx, cz, p.id, p.ws_hits);
        events.push(
            EV_WEAK_MARK,
            p.id,
            ck,
            ((weak_hit as u32) << 8) | next as u32,
        );
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
