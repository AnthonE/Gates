//! Research — the sink OBOL exists for (research v0, DECISIONS.md §open).
//!
//! You find a revolver in a crate. You stand at a research table with it
//! in your hand, press the key, and it is gone along with a fistful of
//! coin — and from then on you can craft revolvers. That is scrap's whole
//! job in the reference game and it is the reason a currency is a
//! currency rather than a trophy: **a faucet with no sink is a number
//! that goes up.**
//!
//! ## The shape, and what it deliberately is not
//!
//! - **A station, not a container.** The table is checked by proximity,
//!   exactly as `craft.rs` checks a workbench, and the thing being
//!   researched comes out of an inventory slot the request names. The
//!   reference puts the item *into* the table and we do not, because a
//!   container costs a box record, a wire handle and a second place for a
//!   stack to be lost in — and the verb reads identically to the player,
//!   who is holding the thing either way.
//! - **The unlock is a bit, and the bit is per player.** `Player::known`
//!   is a `u64` mask over recipe indices ([`crate::limits::KNOWN_MASK_BITS`]
//!   asserts the two agree), so "do you know this" is one shift and one
//!   AND on the craft path — no map, no allocation, nothing to iterate
//!   (walls 1 and 2). It rides `PlayerSave`, so it survives a logout.
//! - **What it costs is content.** A row names an item, what it costs, and
//!   nothing else; the coin itself is `ResearchContent::coin`, an item
//!   index the bake resolves from `content/research.toml`. The sim never
//!   learns that the coin is special, which is the same posture the
//!   recycler takes from the other end — one pays it, one charges it, and
//!   neither knows what it is.
//! - **Two verbs over one mask** (tech tree v0, spoken 2026-08-14, the
//!   pre-Oct-2025 era 2026-08-15). [`research`] is the table: sample in
//!   hand, coin, no prerequisites — research what you loot. [`unlock`] is
//!   the tree: no sample, coin alone, at a bench whose tier covers the
//!   recipe, and only along the graph `ResearchRow::requires` draws —
//!   tech-tree the gaps. The asymmetry is the reference's own and it is
//!   load-bearing: the table bypassing the tree is what keeps a lucky
//!   find worth the walk, and the tree needing no sample is what makes
//!   an item nothing drops reachable at all (the satchel charge was
//!   unlearnable dead content until this verb existed — blueprint-gated,
//!   in no loot table).
//!   The prediction the first version of this header made — "the second
//!   is a content graph on top of these bits rather than a change to
//!   them" — held: `Player::known` did not move, the wire's `Known` mask
//!   did not move, and the tree is `requires` plus one function.
//!
//! Not in this slice, documented rather than forgotten: no blueprint
//! *item* (learning is instant and personal, so there is nothing to trade
//! — the reference's paper blueprints are a separate economy), no wipe
//! schedule (`DESIGN.md` §8 says blueprints outlive a wipe, and there is
//! no wipe mechanism yet to outlive), and no partial refund.

use crate::craft::{inv_count, inv_take};
use crate::deploy::{DeployContent, Deploys, ARCH_RESEARCH};
use crate::gather::ItemStack;
use crate::limits::{INV_SLOTS, MAX_RECIPES, MAX_RESEARCH_ROWS};
use crate::world::{EventQueue, Player, EV_KNOWN, EV_RESEARCH, EV_RESEARCH_REFUSED};

/// How close a placed table must stand, planar metres. The workbench's
/// number and deliberately the same one: two stations with two reaches
/// would be a rule a player has to learn twice.
pub use crate::craft::STATION_RADIUS_M as TABLE_RADIUS_M;

/// Integer refusal reasons (CLAUDE.md wall 3), carried by
/// `EV_RESEARCH_REFUSED` / the research-refused wire subtype.
pub const REFUSE_R_TABLE: u32 = 0;
pub const REFUSE_R_SLOT: u32 = 1;
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
/// The highest live reason, named rather than counted — the domain gate
/// reads this and the wire's field width is bounded against it.
pub const REFUSE_R_MAX: u32 = REFUSE_R_BENCH;

/// One baked research row. `cost == 0` **is** a live row (a thing you may
/// learn for free), so emptiness is `recipe == NO_RECIPE` rather than a
/// zero price — the mistake the cook table's `ticks == 0` avoids by
/// meaning something no live row can mean.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResearchRow {
    /// The item you must be holding, consumed one unit on success.
    pub item: u16,
    /// The recipe index it unlocks — resolved at bake, because the mask is
    /// over recipes and the row is written in terms of the thing a player
    /// recognises.
    pub recipe: u16,
    /// Units of `ResearchContent::coin` the unlock burns.
    pub cost: u16,
    /// The tree edge (tech tree v0): the **recipe index** of the parent
    /// node, or [`NO_RECIPE`] for a root. Resolved at bake from the
    /// parent row's item id, so content writes "requires gunpowder" and
    /// the sim checks one bit of `Player::known`. Only [`unlock`] reads
    /// it — the table verb researches a looted sample with no questions
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
}

impl ResearchContent {
    /// Inert: nothing is researchable, every request refuses.
    /// `World::new` starts here.
    pub const EMPTY: Self = Self {
        rows: [ResearchRow::INERT; MAX_RESEARCH_ROWS],
        row_count: 0,
        coin: 0,
    };

    /// Synthetic table for the parity/replay/alloc gates, over the gather
    /// probe fixture's items. Item 4 unlocks recipe 2 — which is the
    /// craft fixture's station-gated row, so the blueprint refusal and the
    /// station refusal are both reachable on one recipe — and it is paid
    /// in item 3. Row 1 is the tree's probe (tech tree v0): item 5
    /// unlocks recipe 1 **only after recipe 2**, so the parent refusal
    /// and the parent-satisfied unlock are both inside the gates too.
    pub fn probe_fixture() -> Self {
        let mut c = Self::EMPTY;
        c.coin = 3;
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

/// Apply one research request (`Command::Research`).
///
/// Order is the container rule applied to a two-part price (`inventory.rs`:
/// validation before mutation, always): every refusal is decided before
/// anything is taken, so a request that fails costs the player nothing and
/// a request that succeeds takes both halves or neither. The failure this
/// prevents is the one worth naming — charging the coin, finding the item
/// gone, and leaving a player poorer and no wiser.
#[allow(clippy::too_many_arguments)]
pub fn research(
    rc: &ResearchContent,
    dc: &DeployContent,
    deploys: &Deploys,
    p: &mut Player,
    slot: u8,
    events: &mut EventQueue,
) {
    let px = p.body.qx as f32 * crate::movement::POS_XZ_Q;
    let pz = p.body.qz as f32 * crate::movement::POS_XZ_Q;
    if !deploys.arch_near(dc, ARCH_RESEARCH, px, pz, TABLE_RADIUS_M) {
        events.push(EV_RESEARCH_REFUSED, p.id, REFUSE_R_TABLE, 0);
        return;
    }
    let slot = slot as usize;
    if slot >= INV_SLOTS || p.inv[slot].count == 0 {
        events.push(EV_RESEARCH_REFUSED, p.id, REFUSE_R_SLOT, 0);
        return;
    }
    let item = p.inv[slot].item;
    let Some(row) = rc.row_for(item).copied() else {
        events.push(EV_RESEARCH_REFUSED, p.id, REFUSE_R_ITEM, 0);
        return;
    };
    if knows(p.known, row.recipe) {
        events.push(EV_RESEARCH_REFUSED, p.id, REFUSE_R_KNOWN, 0);
        return;
    }
    // The coin, counted across the whole inventory rather than one stack —
    // 75 obol in three stacks is 75 obol. Counted BEFORE the item is taken
    // because the item may itself be the coin: a content set that priced
    // researching a coin in coins would otherwise charge a different
    // number depending on which half ran first.
    // **No prerequisite check here, and that is the design rather than an
    // omission** (merge of 2026-08-15). This verb is the research TABLE:
    // sample in hand, coin, no questions asked. The tree's edges are
    // `unlock`'s to enforce, and the asymmetry is the reference's own —
    // the table bypassing the tree is what keeps a lucky find worth the
    // walk. An earlier local branch put `requires` on this verb instead
    // and its own module header admitted it had built the edge on the
    // first system rather than the second; that half is retired here.
    if inv_count(&p.inv, rc.coin) < row.cost as u32 {
        events.push(EV_RESEARCH_REFUSED, p.id, REFUSE_R_COST, 0);
        return;
    }

    // Past here nothing can fail. Take the sample, then the price.
    p.inv[slot].count -= 1;
    if p.inv[slot].count == 0 {
        p.inv[slot] = ItemStack::default();
    }
    inv_take(&mut p.inv, rc.coin, row.cost as u32);
    p.known |= 1u64 << row.recipe;
    events.push(EV_RESEARCH, p.id, row.recipe as u32, row.cost as u32);
    // And the mask itself, whole, right behind the success. The server
    // used to synthesise this by reaching back into `world.players` when
    // it saw `EV_RESEARCH` go past — which worked, and meant the mask was
    // a fact only the purchase could state. It is a door fact too now
    // (`EV_KNOWN` says why), so it is pushed where every other fact is
    // pushed: by the sim, at the moment it became true. The server's job
    // shrinks to encoding it.
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
/// the second of the two systems, for the things you never looted.
///
/// Refusal order mirrors `craft::enqueue`'s blueprint-before-station
/// call, one level up: the **parent** is reported before the **bench**,
/// because a missing parent is the permanent fact (go unlock something
/// else first) and a missing bench is transient and self-evident (walk
/// to it). The cost comes last, matching [`research`] — and as there,
/// every refusal lands before anything is taken.
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
}
