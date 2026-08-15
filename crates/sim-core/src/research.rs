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
//! - **A tree, as of 2026-08-15, and a shallow one.** A row unlocks one
//!   recipe and may name prerequisites (`ResearchRow::requires`, a mask in
//!   `Player::known`'s own bit space). This line said "not a tech tree …
//!   depends on nothing" until the ladder landed, which was true when it
//!   was written. What is still true is the split: the reference has both a
//!   research table and a tech tree as separate systems, and this is the
//!   first with a dependency edge on it, not the second. The BENCH tier —
//!   workbench 2/3 and the tree UI — is unbuilt and is a spoken knob
//!   (`NOW.md` §0tt, `DECISIONS.md` §open).
//! - **The edges are not free-form.** `validate::structural` refuses a row
//!   that omits a prerequisite the craft graph already implies, so the
//!   authored tree can add to the recipes' own dependencies but never
//!   contradict them.
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
use crate::world::{EventQueue, Player, EV_RESEARCH, EV_RESEARCH_REFUSED};

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
/// A prerequisite blueprint is not held yet (`ResearchRow::requires`).
/// Ordered before the price on purpose: a row you cannot reach must not
/// bill you for finding that out.
pub const REFUSE_R_LOCKED: u32 = 5;
/// The highest live reason, named rather than counted — the domain gate
/// reads this and the wire's field width is bounded against it.
pub const REFUSE_R_MAX: u32 = REFUSE_R_LOCKED;

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
    /// The recipes that must already be known before this row may be
    /// bought — a mask in `Player::known`'s own space, so the check is one
    /// AND and the two can never disagree about what a bit means.
    ///
    /// Zero is the whole table's shape before 2026-08-15 and stays the
    /// default: a root of the tree depends on nothing. The edges are
    /// resolved at bake from item ids, for `recipe`'s reason — the file
    /// names things a player recognises and never an index.
    pub requires: u64,
}

/// The empty row's recipe. Past `MAX_RECIPES` by construction, so an
/// inert row can never be mistaken for one naming recipe 0.
pub const NO_RECIPE: u16 = u16::MAX;

impl ResearchRow {
    pub const INERT: Self = Self {
        item: 0,
        recipe: NO_RECIPE,
        cost: 0,
        requires: 0,
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
    /// in item 3.
    pub fn probe_fixture() -> Self {
        let mut c = Self::EMPTY;
        c.coin = 3;
        c.row_count = 1;
        c.rows[0] = ResearchRow {
            item: 4,
            recipe: 2,
            cost: 5,
            requires: 0,
        };
        c
    }

    /// The row for `item`, if it is researchable at all.
    pub fn row_for(&self, item: u16) -> Option<&ResearchRow> {
        self.rows[..self.row_count as usize]
            .iter()
            .find(|r| r.is_live() && r.item == item)
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
    // The ladder, and it is checked before the price: a row whose
    // prerequisites are unmet is not a thing you failed to afford, and
    // billing the refusal in that order would read as "you are poor" for a
    // door that is closed for another reason entirely.
    if p.known & row.requires != row.requires {
        events.push(EV_RESEARCH_REFUSED, p.id, REFUSE_R_LOCKED, 0);
        return;
    }
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
    fn the_fixture_names_one_row_and_finds_it_by_item() {
        let rc = ResearchContent::probe_fixture();
        let row = rc.row_for(4).expect("the fixture researches item 4");
        assert_eq!((row.recipe, row.cost), (2, 5));
        assert!(rc.row_for(0).is_none(), "and nothing else");
    }
}
