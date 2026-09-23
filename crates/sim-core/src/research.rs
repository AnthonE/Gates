//! Research — the sink JUNK exists for (research v0, DECISIONS.md §open;
//! the timed table and the blueprint item are research table v1).
//!
//! You find a revolver in a crate. You put it in a research table beside a
//! fistful of junk, press the switch, and ten seconds later the revolver is
//! gone and a **blueprint** lies where it was — paper you can carry, trade,
//! drop or lose, and read (`Command::Research`) to make revolvers from then
//! on. That is scrap's whole job in the reference game and it is the reason
//! a currency is a currency rather than a trophy: **a faucet with no sink is
//! a number that goes up.**
//!
//! ## The shape (research table v1, operator 2026-09-22: *"Timed research
//! table"*)
//!
//! - **The table is a container.** v0 argued the opposite — a station
//!   checked by proximity, the sample taken out of the hand — and the
//!   operator's answer overturned it, because the reference's table is a
//!   two-slot box: the item goes IN, the scrap goes in beside it, and the
//!   wait is the point. It is a box record (`deploy::holds_items`) with an
//!   oven state beside it, so `CONT_BOX` opens it, the move verb fills it, a
//!   raid spills it and a world save keeps a research half done — all of it
//!   machinery the fire and the recycler already paid for. Slot 0 takes ONE
//!   unit of something a row researches, slot 1 the coin, the rest nothing
//!   ([`table_accepts`], `inventory::REFUSE_M_TABLE`), and nothing moves
//!   while it runs (`inventory::REFUSE_M_BUSY` — the reference locks its
//!   table too).
//! - **A use press starts it** ([`begin`]) — the switch the fire and the
//!   recycler already take — and the ovens' sweep lands it
//!   ([`table_sweep`]). The coin and the sample are taken at the LANDING,
//!   the reference's order, so a table destroyed mid-research spills both
//!   whole and charges nothing.
//! - **The blueprint is one item for every recipe**, its target in the
//!   stack's `cond` ([`blueprint_of`], [`blueprint_target`]). The reference
//!   does the same — one blueprint item, a per-instance target — and ours
//!   had to: 62 of the 64 item defs are spoken for. `cond` is free to borrow
//!   because the blueprint's ceiling is zero, which `validate` holds it to:
//!   nothing wears a zero-ceiling item, nothing repairs one, no pip is drawn
//!   for one, and stack 1 means two never merge. Every store a stack can
//!   reach — a box, a bag, the ground, a save — carries `cond` whole.
//! - **Reading it is its own verb** ([`study`]): anywhere, no table, no
//!   coin, because both were paid when the paper was made — possibly by
//!   somebody else. Paper for a recipe you already know is REFUSED and KEPT:
//!   it is still worth handing to someone who does not.
//! - **The unlock is a bit, and the bit is per player.** `Player::known`
//!   is a `u64` mask over recipe indices ([`crate::limits::KNOWN_MASK_BITS`]
//!   asserts the two agree), so "do you know this" is one shift and one
//!   AND on the craft path — no map, no allocation, nothing to iterate
//!   (walls 1 and 2). It rides `PlayerSave`, so it survives a logout.
//! - **What it costs is content.** A row names an item, what it costs, and
//!   its tree parent; the coin is `ResearchContent::coin`, the blueprint
//!   `ResearchContent::blueprint` and the wait `ResearchContent::table_ticks`
//!   — item indices and a tick count the bake resolves from
//!   `content/research.toml`. The sim never learns that the coin is special,
//!   which is the same posture the recycler takes from the other end — one
//!   pays it, one charges it, and neither knows what it is.
//! - **Two roads to one mask** (tech tree v0, spoken 2026-08-14). The
//!   table researches what you LOOTED — sample, coin, no prerequisites — and
//!   the tree ([`unlock`]) teaches what you never found: no sample, coin
//!   alone, at a bench whose tier covers the recipe, and only along the
//!   graph `ResearchRow::requires` draws. The asymmetry is the reference's
//!   own and it is load-bearing: the table bypassing the tree is what keeps
//!   a lucky find worth the walk, and the tree needing no sample is what
//!   makes an item nothing drops reachable at all. The tree stays instant
//!   and item-less, as theirs is.
//!
//! Not in this slice, documented rather than forgotten: no wipe schedule
//! (`DESIGN.md` §8 says blueprints outlive a wipe, and there is no wipe
//! mechanism yet to outlive), no partial refund, and no crafter that reads
//! paper without learning it (the reference's Industrial Crafter).

use crate::craft::{inv_count, inv_take};
use crate::deploy::{box_key, DeployContent, Deploys};
use crate::gather::{ItemStack, NO_ITEM};
use crate::limits::{INV_SLOTS, MAX_RECIPES, MAX_RESEARCH_ROWS};
use crate::oven::OVEN_PERIOD_TICKS;
use crate::world::{EventQueue, Player, EV_KNOWN, EV_RESEARCH, EV_RESEARCH_REFUSED};

/// How close a placed bench must stand for the tree verb, planar metres.
/// The workbench's number and deliberately the same one: two stations with
/// two reaches would be a rule a player has to learn twice. (The research
/// table itself is a container since v1 and answers to a box's reach,
/// `Deploys::box_in_reach`.)
pub use crate::craft::STATION_RADIUS_M as TABLE_RADIUS_M;

/// Integer refusal reasons (CLAUDE.md wall 3), carried by
/// `EV_RESEARCH_REFUSED` / the research-refused wire subtype.
///
/// No research table at arm's length of the press.
pub const REFUSE_R_TABLE: u32 = 0;
/// Nothing to work on: the table's item slot is empty, or the inventory
/// slot a study names holds nothing.
pub const REFUSE_R_SLOT: u32 = 1;
/// Not something this verb can use: a table sample no row researches, or
/// paper that is not a blueprint, is blank, or teaches what the table no
/// longer researches.
pub const REFUSE_R_ITEM: u32 = 2;
pub const REFUSE_R_KNOWN: u32 = 3;
pub const REFUSE_R_COST: u32 = 4;
/// The tree verb's prerequisite: the row's `requires` parent is not yet
/// learned (tech tree v0). Its own reason rather than `REFUSE_R_ITEM`
/// because the two send a player opposite ways — one says *this cannot
/// be researched*, this says *unlock the node before it first*.
pub const REFUSE_R_PARENT: u32 = 5;
/// The tree verb's station: no workbench of the recipe's tier in reach.
/// The tree's own `REFUSE_R_TABLE` — a different building, so a
/// different sentence.
pub const REFUSE_R_BENCH: u32 = 6;
/// The table's clock (research table v1): a research is already running
/// at this table. Its own reason because its remedy is the one no other
/// refusal gives — wait.
pub const REFUSE_R_BUSY: u32 = 7;
/// The highest live reason, named rather than counted — the domain gate
/// reads this and the wire's field width is bounded against it (three
/// bits, so this is the last reason that fits without a turn).
pub const REFUSE_R_MAX: u32 = REFUSE_R_BUSY;

/// The research table's two working slots (research table v1). The table
/// is a box record, `BOX_SLOTS` wide like every container, and every slot
/// past these two refuses everything ([`table_accepts`]).
///
/// Slot 0 holds the thing being researched — one unit — and, once the
/// research lands, the blueprint it became.
pub const TABLE_ITEM_SLOT: usize = 0;
/// Slot 1 holds the coin the research is paid from.
pub const TABLE_COIN_SLOT: usize = 1;

/// One baked research row. `cost == 0` **is** a live row (a thing you may
/// learn for free), so emptiness is `recipe == NO_RECIPE` rather than a
/// zero price — the mistake the cook table's `ticks == 0` avoids by
/// meaning something no live row can mean.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResearchRow {
    /// The item a table consumes, one unit, into this row's blueprint.
    pub item: u16,
    /// The recipe index it unlocks — resolved at bake, because the mask is
    /// over recipes and the row is written in terms of the thing a player
    /// recognises.
    pub recipe: u16,
    /// Units of `ResearchContent::coin` the table and the tree each charge.
    pub cost: u16,
    /// The tree edge (tech tree v0): the **recipe index** of the parent
    /// node, or [`NO_RECIPE`] for a root. Resolved at bake from the
    /// parent row's item id, so content writes "requires pistol ammo" and
    /// the sim checks one bit of `Player::known`. Only [`unlock`] reads
    /// it — the table researches a looted sample with no questions
    /// asked, which is the two-system split the reference runs.
    pub requires: u16,
}

/// The empty row's recipe. Past `MAX_RECIPES` by construction, so an
/// inert row can never be mistaken for one naming recipe 0.
pub const NO_RECIPE: u16 = u16::MAX;

impl ResearchRow {
    pub const INERT: Self = Self {
        item: 0,
        recipe: NO_RECIPE,
        cost: 0,
        requires: NO_RECIPE,
    };

    pub fn is_live(&self) -> bool {
        (self.recipe as usize) < MAX_RECIPES
    }
}

/// The whole research ruleset, baked from `content/research.toml`.
/// Construction input like every other content table: installed before the
/// first tick, hashed into the WAL header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResearchContent {
    pub rows: [ResearchRow; MAX_RESEARCH_ROWS],
    pub row_count: u16,
    /// What research is paid in. An item index and nothing more — the sim
    /// does not know this is a currency, which is what keeps `DESIGN.md`
    /// §3.1's coin out of `crates/`.
    pub coin: u16,
    /// The blueprint item (research table v1): what a finished research
    /// leaves in the table, its target in `cond`. `NO_ITEM` when the
    /// content names none, and then no table ever starts.
    pub blueprint: u16,
    /// Ticks one research takes (`[table] seconds` × `TICK_HZ`). Zero is a
    /// table that never starts — `blueprint == NO_ITEM`'s inert posture,
    /// and what `World::new` holds.
    pub table_ticks: u16,
}

impl ResearchContent {
    /// Inert: nothing is researchable, every request refuses.
    /// `World::new` starts here.
    pub const EMPTY: Self = Self {
        rows: [ResearchRow::INERT; MAX_RESEARCH_ROWS],
        row_count: 0,
        coin: 0,
        blueprint: NO_ITEM,
        table_ticks: 0,
    };

    /// Synthetic table for the parity/replay/alloc gates, over the gather
    /// probe fixture's items. Item 4 unlocks recipe 2 — which is the
    /// craft fixture's station-gated row, so the blueprint refusal and the
    /// station refusal are both reachable on one recipe — and it is paid
    /// in item 3. Row 1 is the tree's probe (tech tree v0): item 5
    /// unlocks recipe 1 **only after recipe 2**, so the parent refusal
    /// and the parent-satisfied unlock are both inside the gates too.
    ///
    /// The paper is item 11, the gather fixture's one stack-of-one item,
    /// and a research takes 30 ticks — two of the ovens' 15-tick periods,
    /// so the sweep's advance and its landing are both a visit apart.
    pub fn probe_fixture() -> Self {
        let mut c = Self::EMPTY;
        c.coin = 3;
        c.blueprint = 11;
        c.table_ticks = 30;
        c.row_count = 2;
        c.rows[0] = ResearchRow {
            item: 4,
            recipe: 2,
            cost: 5,
            requires: NO_RECIPE,
        };
        c.rows[1] = ResearchRow {
            item: 5,
            recipe: 1,
            cost: 4,
            requires: 2,
        };
        c
    }

    /// The row for `item`, if it is researchable at all.
    pub fn row_for(&self, item: u16) -> Option<&ResearchRow> {
        self.rows[..self.row_count as usize]
            .iter()
            .find(|r| r.is_live() && r.item == item)
    }

    /// The row that unlocks `recipe`, if any — the tree verb's lookup,
    /// where the request names the node rather than a held sample.
    pub fn row_for_recipe(&self, recipe: u16) -> Option<&ResearchRow> {
        self.rows[..self.row_count as usize]
            .iter()
            .find(|r| r.is_live() && r.recipe == recipe)
    }
}

/// Does `known` carry the bit for `recipe`?
///
/// The one place the shift is written. A recipe past the mask's width
/// answers **false** rather than wrapping: `1u64 << 64` is undefined-shaped
/// arithmetic in every language that has bitten anyone, and the cap assert
/// in `limits.rs` means a live recipe can never reach here anyway — so
/// this is the belt to that braces.
#[inline]
pub fn knows(known: u64, recipe: u16) -> bool {
    (recipe as usize) < crate::limits::KNOWN_MASK_BITS && known & (1u64 << recipe) != 0
}

/// The stack a finished research leaves behind: one blueprint teaching
/// `target` (research table v1).
///
/// `cond` is the target **plus one**, so zero — what every mint of a
/// zero-ceiling item writes (a loot roll, a craft, a kit, a cook) — is a
/// blank that teaches nothing. That is the whole reason for the offset: a
/// blueprint minted by any road but a table's can never teach item 0.
pub fn blueprint_of(rc: &ResearchContent, target: u16) -> ItemStack {
    ItemStack {
        item: rc.blueprint,
        count: 1,
        cond: target.saturating_add(1),
    }
}

/// What a stack teaches, if it is a blueprint with a target: the item
/// its research consumed. `None` for an empty slot, another item, a blank
/// — and for anything at all while the content names no blueprint.
///
/// Total, and deliberately not validating the target: an item index no
/// row researches is [`study`]'s refusal to give, in its own words.
pub fn blueprint_target(rc: &ResearchContent, s: ItemStack) -> Option<u16> {
    (s.count > 0 && rc.blueprint != NO_ITEM && s.item == rc.blueprint && s.cond > 0)
        .then(|| s.cond - 1)
}

/// May slot `slot` of a research table hold `s` once a move lands
/// (research table v1)? The move verb asks it of **both** landing sites,
/// because a swap writes the destination's old stack into the source
/// (`World::move_item`, the wear check's argument).
///
/// Slot 0 takes **one** unit of something a row researches — one, because
/// a research consumes one and a remainder would be a second thing the
/// slot had to hold — and slot 1 the coin, any amount. The paper a
/// research leaves in slot 0 goes OUT through this check (an emptied slot
/// is always fine) and never back IN: it is a blueprint, not a sample.
/// Every other slot holds nothing.
pub fn table_accepts(rc: &ResearchContent, slot: u8, s: ItemStack) -> bool {
    if s.count == 0 {
        return true;
    }
    match slot as usize {
        TABLE_ITEM_SLOT => s.count == 1 && s.item != rc.blueprint && rc.row_for(s.item).is_some(),
        TABLE_COIN_SLOT => s.item == rc.coin,
        _ => false,
    }
}

/// The row a loaded table would research, or the reason it would not —
/// [`begin`]'s three content refusals, asked again by [`table_sweep`] at
/// the landing, so the two cannot disagree about what a loaded table is.
///
/// A table whose content names no blueprint or no wait researches nothing
/// (`REFUSE_R_ITEM`): the inert posture, and the one a content swap that
/// retired the paper leaves a running table in.
fn loaded_row(rc: &ResearchContent, items: &[ItemStack]) -> Result<ResearchRow, u32> {
    let sample = items[TABLE_ITEM_SLOT];
    if sample.count == 0 {
        return Err(REFUSE_R_SLOT);
    }
    let row = match rc.row_for(sample.item) {
        Some(r) if sample.count == 1 && rc.blueprint != NO_ITEM && rc.table_ticks > 0 => *r,
        _ => return Err(REFUSE_R_ITEM),
    };
    let coin = items[TABLE_COIN_SLOT];
    let held = if coin.item == rc.coin { coin.count } else { 0 };
    if held < row.cost {
        return Err(REFUSE_R_COST);
    }
    Ok(row)
}

/// Start a research at the table at the address (`Command::Use`, research
/// table v1). Returns false when no research table stands there, so
/// `world.rs` routes the press on to the oven and the door verbs, exactly
/// as `oven::toggle` does.
///
/// **Nothing is taken here.** The price is checked now and charged when
/// the research lands ([`table_sweep`]), which is the reference's order
/// and the one that makes a destroyed table cost nothing: what is in it
/// spills whole. The table is locked while it runs
/// (`inventory::REFUSE_M_BUSY`), so what was checked here is what will be
/// there then — and the landing checks again anyway, because a content
/// swap can move a price under a running table.
///
/// Refusals, in order, each announced and none costing anything: out of
/// reach (`REFUSE_R_TABLE`, a box's own reach), already running
/// (`REFUSE_R_BUSY`), nothing in the item slot (`REFUSE_R_SLOT`), a sample
/// no row researches (`REFUSE_R_ITEM`), and too little coin in the coin
/// slot (`REFUSE_R_COST`). **Knowing the recipe already is not a
/// refusal**: the reference's table exists to make paper, and paper for a
/// recipe you know is the paper you hand a teammate.
pub fn begin(
    rc: &ResearchContent,
    deploys: &mut Deploys,
    p: &Player,
    cx: u16,
    cz: u16,
    level: u8,
    events: &mut EventQueue,
) -> bool {
    let Some(i) = deploys.table_index(box_key(cx, cz, level)) else {
        return false;
    };
    if !deploys.box_in_reach(i, p) {
        events.push(EV_RESEARCH_REFUSED, p.id, REFUSE_R_TABLE, 0);
        return true;
    }
    let (boxes, states) = deploys.oven_parts_mut();
    if states[i].lit {
        events.push(EV_RESEARCH_REFUSED, p.id, REFUSE_R_BUSY, 0);
        return true;
    }
    if let Err(why) = loaded_row(rc, &boxes[i].items) {
        events.push(EV_RESEARCH_REFUSED, p.id, why, 0);
        return true;
    }
    states[i].lit = true;
    states[i].cook[TABLE_ITEM_SLOT] = 0;
    crate::oven::announce(boxes[i].cx, boxes[i].cz, boxes[i].level, true, p.id, events);
    true
}

/// Advance every running research table whose turn this tick is, and land
/// the ones that are done — on the ovens' stride (`OVEN_PERIOD_TICKS`: the
/// same bounded slice of the same store and the same whole-period clock,
/// so no tick visits more than `ceil(MAX_BOXES / OVEN_PERIOD_TICKS)`
/// containers between the two sweeps).
///
/// The wait rides `OvenState::cook[0]` — the oven's per-slot timer, on the
/// slot being worked — so a research half done is saved, loaded and
/// hashed by the code that already saves, loads and hashes a fire.
///
/// A landing re-reads the table ([`loaded_row`]) and then either pays —
/// the coin leaves slot 1 and the sample in slot 0 becomes its blueprint,
/// two writes to one record with nothing between them to fail — or, when
/// a content swap moved the ground under it, stops and takes nothing.
/// Either way the table goes quiet with the snuff's own announcement,
/// `EV_OVEN` with no actor, which is how a panel learns it may be emptied.
pub fn table_sweep(
    rc: &ResearchContent,
    deploys: &mut Deploys,
    tick: u64,
    events: &mut EventQueue,
) {
    let period = OVEN_PERIOD_TICKS;
    let phase = tick % period;
    let (boxes, states) = deploys.oven_parts_mut();
    for i in (phase as usize..states.len()).step_by(period as usize) {
        if states[i].arch != crate::deploy::ARCH_RESEARCH || !states[i].lit {
            continue;
        }
        let waited = states[i].cook[TABLE_ITEM_SLOT].saturating_add(period as u16);
        states[i].cook[TABLE_ITEM_SLOT] = waited;
        if waited < rc.table_ticks {
            continue;
        }
        if let Ok(row) = loaded_row(rc, &boxes[i].items) {
            let sample = boxes[i].items[TABLE_ITEM_SLOT].item;
            let coin = &mut boxes[i].items[TABLE_COIN_SLOT];
            // `loaded_row` proved the slot holds at least the cost when
            // the cost is not zero, and a zero cost takes nothing — so this
            // never underflows and never takes a stack that is not coin.
            coin.count -= row.cost;
            if coin.count == 0 {
                *coin = ItemStack::default();
            }
            boxes[i].items[TABLE_ITEM_SLOT] = blueprint_of(rc, sample);
        }
        states[i].lit = false;
        states[i].cook[TABLE_ITEM_SLOT] = 0;
        crate::oven::announce(boxes[i].cx, boxes[i].cz, boxes[i].level, false, 0, events);
    }
}

/// Learn the blueprint in inventory `slot` (`Command::Research`, research
/// table v1) — reading the paper a table made. Anywhere, with no table and
/// no coin: both were paid when the paper was made.
///
/// Refusals, each announced and none taking the paper: nothing in the slot
/// (`REFUSE_R_SLOT`); not a blueprint, a blank one, or one teaching what no
/// row researches any more (`REFUSE_R_ITEM`); and paper for a recipe this
/// player already knows (`REFUSE_R_KNOWN`) — which **keeps** it, because
/// known paper is still worth handing to somebody who does not know it.
///
/// `EV_RESEARCH.c` is the coin THIS verb burned, which is none: whoever ran
/// the table paid, and the announcement says what the reader was charged.
pub fn study(rc: &ResearchContent, p: &mut Player, slot: u8, events: &mut EventQueue) {
    let slot = slot as usize;
    if slot >= INV_SLOTS || p.inv[slot].count == 0 {
        events.push(EV_RESEARCH_REFUSED, p.id, REFUSE_R_SLOT, 0);
        return;
    }
    let Some(row) = blueprint_target(rc, p.inv[slot])
        .and_then(|t| rc.row_for(t))
        .copied()
    else {
        events.push(EV_RESEARCH_REFUSED, p.id, REFUSE_R_ITEM, 0);
        return;
    };
    if knows(p.known, row.recipe) {
        events.push(EV_RESEARCH_REFUSED, p.id, REFUSE_R_KNOWN, 0);
        return;
    }

    // Past here nothing can fail. One sheet read, the bit set.
    p.inv[slot].count -= 1;
    if p.inv[slot].count == 0 {
        p.inv[slot] = ItemStack::default();
    }
    p.known |= 1u64 << row.recipe;
    events.push(EV_RESEARCH, p.id, row.recipe as u32, 0);
    // And the mask itself, whole, right behind the success — pushed where
    // every other fact is pushed: by the sim, at the moment it became true
    // (`EV_KNOWN` says why a door restates it rather than a delta).
    events.push(EV_KNOWN, p.id, p.known as u32, (p.known >> 32) as u32);
}

/// The bench tier [`unlock`] demands for a node: the recipe's own
/// station code where that code is a bench rung, and tier 1 otherwise —
/// a hand-craftable or furnace-cooked blueprint still unlocks at a
/// bench, because the tree is a thing you walk at a workbench and a node
/// with no bench at all would be a verb with no address. The one place
/// the mapping is written; the client's tree panel groups by the same
/// call so the drawn rung and the demanded rung cannot drift.
#[inline]
pub fn node_tier(station: u8) -> u8 {
    use crate::craft::{STATION_WORKBENCH1, STATION_WORKBENCH3};
    if (STATION_WORKBENCH1..=STATION_WORKBENCH3).contains(&station) {
        station
    } else {
        STATION_WORKBENCH1
    }
}

/// Apply one tech-tree unlock (`Command::Unlock`, tech tree v0): learn
/// `recipe` at a workbench, paying the row's coin, no sample needed —
/// the second of the two roads, for the things you never looted.
///
/// Refusal order mirrors `craft::enqueue`'s blueprint-before-station
/// call, one level up: the **parent** is reported before the **bench**,
/// because a missing parent is the permanent fact (go unlock something
/// else first) and a missing bench is transient and self-evident (walk
/// to it). The cost comes last, as at the table — and as there, every
/// refusal lands before anything is taken.
pub fn unlock(
    rc: &ResearchContent,
    cc: &crate::craft::CraftContent,
    dc: &DeployContent,
    deploys: &Deploys,
    p: &mut Player,
    recipe: u16,
    events: &mut EventQueue,
) {
    let Some(row) = rc.row_for_recipe(recipe).copied() else {
        // Not a node: the recipe exists but the tree does not teach it
        // (or the recipe does not exist at all — one answer, because a
        // forged index and an ungated recipe deserve the same silence
        // about which they were).
        events.push(EV_RESEARCH_REFUSED, p.id, REFUSE_R_ITEM, 0);
        return;
    };
    if knows(p.known, row.recipe) {
        events.push(EV_RESEARCH_REFUSED, p.id, REFUSE_R_KNOWN, 0);
        return;
    }
    if row.requires != NO_RECIPE && !knows(p.known, row.requires) {
        events.push(EV_RESEARCH_REFUSED, p.id, REFUSE_R_PARENT, 0);
        return;
    }
    let tier = node_tier(cc.recipes[row.recipe as usize].station);
    let px = p.body.qx as f32 * crate::movement::POS_XZ_Q;
    let pz = p.body.qz as f32 * crate::movement::POS_XZ_Q;
    if !deploys.bench_near(dc, tier, px, pz, TABLE_RADIUS_M) {
        events.push(EV_RESEARCH_REFUSED, p.id, REFUSE_R_BENCH, 0);
        return;
    }
    if inv_count(&p.inv, rc.coin) < row.cost as u32 {
        events.push(EV_RESEARCH_REFUSED, p.id, REFUSE_R_COST, 0);
        return;
    }

    // Past here nothing can fail. The tree takes only the price.
    inv_take(&mut p.inv, rc.coin, row.cost as u32);
    p.known |= 1u64 << row.recipe;
    events.push(EV_RESEARCH, p.id, row.recipe as u32, row.cost as u32);
    // The mask, whole, behind the success — the table's own statement,
    // and missing here until 2026-09-22. `client-core` takes `SUB_KNOWN`
    // as the authority and deliberately never sets a bit off
    // `EV_RESEARCH`, so without this line a tree unlock was a purchase the
    // client never heard: the node stayed locked, its children stayed
    // blocked and the craft panel kept the recipe LOCKED until some later
    // door (a respawn, a reconnect, a study) restated the mask.
    events.push(EV_KNOWN, p.id, p.known as u32, (p.known >> 32) as u32);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mask_is_one_bit_per_recipe_and_stops_at_its_width() {
        assert!(!knows(0, 0));
        assert!(knows(1, 0));
        assert!(knows(1 << 63, 63));
        assert!(!knows(1 << 63, 62));
        // Past the width: false, never a wrapped shift into a live bit.
        assert!(!knows(u64::MAX, 64));
        assert!(!knows(u64::MAX, NO_RECIPE));
    }

    #[test]
    fn the_empty_table_teaches_nothing() {
        let rc = ResearchContent::EMPTY;
        assert!(rc.row_for(0).is_none());
        assert!(!ResearchRow::INERT.is_live());
        assert_eq!(rc.blueprint, NO_ITEM, "and makes no paper");
        assert_eq!(rc.table_ticks, 0);
    }

    #[test]
    fn the_fixture_finds_its_rows_by_item_and_by_recipe() {
        let rc = ResearchContent::probe_fixture();
        let row = rc.row_for(4).expect("the fixture researches item 4");
        assert_eq!((row.recipe, row.cost, row.requires), (2, 5, NO_RECIPE));
        let node = rc.row_for_recipe(1).expect("recipe 1 is the tree probe");
        assert_eq!((node.item, node.requires), (5, 2));
        assert!(rc.row_for(0).is_none(), "and nothing else");
        assert!(rc.row_for_recipe(0).is_none());
    }

    #[test]
    fn a_node_tier_is_the_bench_rung_or_the_first_one() {
        use crate::craft::{
            STATION_FURNACE, STATION_NONE, STATION_WORKBENCH1, STATION_WORKBENCH2,
            STATION_WORKBENCH3,
        };
        assert_eq!(node_tier(STATION_NONE), STATION_WORKBENCH1);
        assert_eq!(node_tier(STATION_FURNACE), STATION_WORKBENCH1);
        assert_eq!(node_tier(STATION_WORKBENCH1), STATION_WORKBENCH1);
        assert_eq!(node_tier(STATION_WORKBENCH2), STATION_WORKBENCH2);
        assert_eq!(node_tier(STATION_WORKBENCH3), STATION_WORKBENCH3);
    }

    /// The paper round-trips its target, including item 0 — the case the
    /// plus-one exists for — and a zero `cond` is a blank, never item 0.
    #[test]
    fn a_blueprint_names_its_target_and_a_blank_names_nothing() {
        let rc = ResearchContent::probe_fixture();
        for target in [0u16, 4, 63] {
            let paper = blueprint_of(&rc, target);
            assert_eq!((paper.item, paper.count), (rc.blueprint, 1));
            assert_eq!(blueprint_target(&rc, paper), Some(target));
        }
        let blank = ItemStack {
            item: rc.blueprint,
            count: 1,
            cond: 0,
        };
        assert_eq!(
            blueprint_target(&rc, blank),
            None,
            "a blank teaches nothing"
        );
        let other = ItemStack {
            item: 4,
            count: 1,
            cond: 5,
        };
        assert_eq!(blueprint_target(&rc, other), None, "only paper is paper");
        assert_eq!(
            blueprint_target(&ResearchContent::EMPTY, blueprint_of(&rc, 4)),
            None,
            "and content with no blueprint reads no stack as one"
        );
    }

    /// Each slot's rule, including the two that are easy to get backwards:
    /// a second unit of a sample, and the paper going back in.
    #[test]
    fn the_table_slots_take_only_their_own_items() {
        let rc = ResearchContent::probe_fixture();
        let one = |item, count| ItemStack {
            item,
            count,
            cond: 0,
        };
        assert!(table_accepts(&rc, 0, one(4, 1)), "one researchable unit");
        assert!(!table_accepts(&rc, 0, one(4, 2)), "never two");
        assert!(
            !table_accepts(&rc, 0, one(3, 1)),
            "the coin is not a sample"
        );
        assert!(
            !table_accepts(&rc, 0, blueprint_of(&rc, 4)),
            "paper goes out of the table and never back in"
        );
        assert!(table_accepts(&rc, 1, one(3, 90)), "any amount of coin");
        assert!(!table_accepts(&rc, 1, one(4, 1)), "and only coin");
        assert!(!table_accepts(&rc, 2, one(3, 1)), "the rest hold nothing");
        for slot in 0..4 {
            assert!(
                table_accepts(&rc, slot, ItemStack::default()),
                "an emptied slot is always fine — that is how things come out"
            );
        }
    }
}
