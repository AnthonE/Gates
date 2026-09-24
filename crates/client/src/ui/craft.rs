//! The craft panel's model — the reference `crafting.png`, read as a list of
//! things a panel must be able to answer.
//!
//! The reference frame answers six questions at once and ours answered one.
//! `MENUS.md` §3 says so in as many words: *"flat list, click to enqueue,
//! shift-click ×5. The reference frame has categories, search, a detail pane
//! with craft time and workbench requirement, an AMOUNT/ITEM TYPE/TOTAL/HAVE
//! ingredient table, favourites and a quantity stepper."* Every one of those
//! is arithmetic over tables the client already holds, which is why it lives
//! here and not in a system.
//!
//! ## The category rail is ours, not the reference's, and that is deliberate
//!
//! The reference rail sorts by **item class** — CONSTRUCTION, RESOURCES,
//! CLOTHING, TOOLS, MEDICAL, WEAPONS, AMMO. We cannot draw that rail
//! honestly, because **the wire does not carry an item's class.**
//! `EventMsg::Catalog` ships display names, condition ceilings (v46) and
//! the two armor columns (v52) — no class among them (`protocol/src/
//! event.rs`), so a client-side class would be a guess made from a string,
//! and a guess in a filter is a recipe the player cannot find. Note what
//! the armor columns are *not*: `wear_slot` says where a piece is worn,
//! which is CLOTHING for the three rows that carry it and silent about
//! every other item, so it is one bucket of seven and not a rail.
//!
//! So the rail here is built from what the client provably knows, and every
//! bucket is a fact rather than a category: which station a recipe needs
//! (`RecipeDef::station`), whether its output places a deployable
//! (`DeployDef::item`), whether its output feeds another recipe
//! (`RecipeDef::inputs`), and which rows the player starred (a local latch,
//! exactly as the reference's own FAVOURITE is). Giving us the reference's
//! rail is a one-field content and wire change — a class byte per item in
//! the catalog message, a `PROTO_VER` bump and regenerated goldens in the
//! same commit (wall 6) — and `NOW.md` carries the ask rather than this file
//! faking it.

use protocol::event::ItemCatalog;
use sim_core::craft::{
    inv_count, CraftContent, RecipeDef, STATION_FURNACE, STATION_NONE, STATION_WORKBENCH1,
    STATION_WORKBENCH2, STATION_WORKBENCH3,
};
use sim_core::deploy::DeployContent;
use sim_core::gather::ItemStack;
use sim_core::limits::{INV_SLOTS, MAX_RECIPE_INPUTS, TICK_HZ};

/// A bucket on the left rail. Each is computable from a table the client
/// holds — see the module note for why it is not the reference's rail.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cat {
    /// Rows the player starred. A local latch; nothing on the wire.
    Favourite,
    All,
    /// `STATION_NONE` — craftable standing in a field.
    ByHand,
    Workbench,
    Furnace,
    /// The output places something (`DeployDef::item`).
    Deployable,
    /// The output is an input of another recipe.
    Component,
}

/// The rail, top to bottom. `Favourite` first and `All` second, which is the
/// reference frame's own order.
pub const RAIL: [Cat; 7] = [
    Cat::Favourite,
    Cat::All,
    Cat::ByHand,
    Cat::Workbench,
    Cat::Furnace,
    Cat::Deployable,
    Cat::Component,
];

impl Cat {
    pub fn label(self) -> &'static str {
        match self {
            Cat::Favourite => "FAVOURITE",
            Cat::All => "ALL",
            Cat::ByHand => "BY HAND",
            Cat::Workbench => "WORKBENCH",
            Cat::Furnace => "FURNACE",
            Cat::Deployable => "DEPLOYABLE",
            Cat::Component => "COMPONENT",
        }
    }
}

/// What the panel knows about the tables, computed once when they change
/// rather than per row per frame.
///
/// `deployable` and `component` are the two buckets that need a pass over
/// another table to decide, and both are stable for as long as the content
/// is — which is the whole session, since the content hash is pinned into
/// the WAL header. Recomputing them per frame would be a scan of
/// `MAX_RECIPES × MAX_RECIPE_INPUTS` for a set that cannot have changed.
#[derive(Clone, Copy, Debug)]
pub struct Facts {
    deployable: [bool; MAX_ITEMS],
    component: [bool; MAX_ITEMS],
}

/// Ceiling on item indices this panel indexes by. `u16` on the wire, but
/// the baked tables are far smaller and a fixed array keeps the fact
/// lookups allocation-free; an index past it simply answers `false`, which
/// is the honest answer for an item no baked table mentions.
pub const MAX_ITEMS: usize = 256;

impl Default for Facts {
    fn default() -> Self {
        Self {
            deployable: [false; MAX_ITEMS],
            component: [false; MAX_ITEMS],
        }
    }
}

impl Facts {
    /// Rebuild from the two tables. Cheap and rare: called when a def drip
    /// lands, never per frame.
    pub fn build(recipes: &CraftContent, deploys: &DeployContent) -> Self {
        let mut f = Self::default();
        for d in deploys.defs.iter().take(deploys.def_count as usize) {
            if d.hp > 0 {
                if let Some(slot) = f.deployable.get_mut(d.item as usize) {
                    *slot = true;
                }
            }
        }
        for r in recipes.recipes.iter().take(recipes.recipe_count as usize) {
            if r.out_count == 0 {
                continue;
            }
            for (item, units) in r.inputs.iter().take(r.n_inputs as usize) {
                if *units > 0 {
                    if let Some(slot) = f.component.get_mut(*item as usize) {
                        *slot = true;
                    }
                }
            }
        }
        f
    }

    pub fn is_deployable(&self, item: u16) -> bool {
        self.deployable.get(item as usize).copied().unwrap_or(false)
    }

    pub fn is_component(&self, item: u16) -> bool {
        self.component.get(item as usize).copied().unwrap_or(false)
    }
}

/// One row of the recipe grid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row {
    pub recipe: u16,
    pub output: u16,
    pub out_count: u16,
    pub station: u8,
    /// Whole crafts the current inventory pays for. Zero draws the row
    /// dimmed — the reference greys an unaffordable recipe rather than
    /// hiding it, so the player can see what to go and get.
    pub affordable: u32,
    /// Blueprint-gated and not yet learned (research v0). **Shown, not
    /// hidden**, for `affordable`'s reason exactly: a recipe you cannot
    /// craft yet is the thing that tells you what a research table is
    /// for, and hiding it would make the whole verb invisible to a player
    /// who has never seen one.
    pub locked: bool,
}

/// Does this recipe belong in `cat`?
pub fn in_category(cat: Cat, recipe: u16, def: &RecipeDef, facts: &Facts, favs: &[u16]) -> bool {
    match cat {
        Cat::Favourite => favs.contains(&recipe),
        Cat::All => true,
        Cat::ByHand => def.station == STATION_NONE,
        // Any rung: the bucket answers "do I need a bench", and which
        // level is the badge's job (bench ladder v0). An equality against
        // rung 1 here is how a tier-2 recipe silently vanishes from every
        // station bucket.
        Cat::Workbench => (STATION_WORKBENCH1..=STATION_WORKBENCH3).contains(&def.station),
        Cat::Furnace => def.station == STATION_FURNACE,
        Cat::Deployable => facts.is_deployable(def.output),
        Cat::Component => facts.is_component(def.output),
    }
}

/// Case-insensitive ASCII substring, over the catalog's raw name bytes.
///
/// Bytes rather than `str` on purpose: the catalog stores `[u8; N]` with a
/// length, a name is not guaranteed to be UTF-8 by anything on the wire, and
/// a lossy conversion per row per keystroke would allocate in a filter. An
/// empty query matches everything, which is what an empty search box means.
pub fn name_matches(name: &[u8], query: &str) -> bool {
    let q = query.trim().as_bytes();
    if q.is_empty() {
        return true;
    }
    if q.len() > name.len() {
        return false;
    }
    let eq = |a: u8, b: u8| a.eq_ignore_ascii_case(&b);
    name.windows(q.len())
        .any(|w| w.iter().zip(q.iter()).all(|(a, b)| eq(*a, *b)))
}

/// How many whole crafts the inventory pays for. `u32` because 30 slots of
/// `u16` overflow a `u16`, and saturating rather than wrapping because the
/// only thing downstream of a wrapped ceiling is a stepper that offers a
/// craft the player cannot pay for.
pub fn affordable(def: &RecipeDef, inv: &[ItemStack; INV_SLOTS]) -> u32 {
    if def.out_count == 0 {
        return 0;
    }
    let mut least = u32::MAX;
    for (item, units) in def.inputs.iter().take(def.n_inputs as usize) {
        if *units == 0 {
            continue;
        }
        least = least.min(inv_count(inv, *item) / *units as u32);
    }
    // A recipe with no inputs is not free forever — the sim still gates it —
    // but there is nothing here to divide by, so the panel offers one.
    if least == u32::MAX {
        1
    } else {
        least
    }
}

/// Fill `out` with the rows of `cat` matching `query`, ordered by recipe
/// index (the bake's order, which is `content/recipes.toml`'s order).
///
/// Takes a `&mut Vec` rather than returning one: the panel calls this
/// whenever the filter or the inventory changes, and reusing the buffer
/// keeps a keystroke from allocating. `CLAUDE.md`'s zero-allocation wall is
/// the sim thread's, not this one's — but "the client is a hot path too" is
/// on the same list, and a filter is exactly where a per-keystroke `Vec`
/// would go unnoticed.
// Eight, and every one is a table the filter genuinely reads: two content
// tables, the inventory, the catalog, the derived facts, the favourites, the
// bucket and the query. A struct wrapping them would be a struct built at
// every call site to be destructured here.
#[allow(clippy::too_many_arguments)]
pub fn rows(
    recipes: &CraftContent,
    inv: &[ItemStack; INV_SLOTS],
    catalog: &ItemCatalog,
    facts: &Facts,
    favs: &[u16],
    known: u64,
    cat: Cat,
    query: &str,
    out: &mut Vec<Row>,
) {
    out.clear();
    for (i, def) in recipes
        .recipes
        .iter()
        .take(recipes.recipe_count as usize)
        .enumerate()
    {
        // `out_count == 0` is the inert row the empty table is full of.
        if def.out_count == 0 {
            continue;
        }
        let recipe = i as u16;
        if !in_category(cat, recipe, def, facts, favs) {
            continue;
        }
        if !name_matches(catalog.name(def.output as usize), query) {
            continue;
        }
        out.push(Row {
            recipe,
            output: def.output,
            out_count: def.out_count,
            station: def.station,
            affordable: affordable(def, inv),
            // The sim's own predicate, imported rather than re-derived:
            // one shift written once, so a client cannot disagree with the
            // server about what a player knows.
            locked: def.blueprint && !sim_core::research::knows(known, recipe),
        });
    }
}

/// One line of the detail pane's ingredient table — the reference frame's
/// AMOUNT / ITEM TYPE / TOTAL / HAVE, in that order and named for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ingredient {
    /// AMOUNT: units one craft consumes.
    pub amount: u16,
    /// ITEM TYPE: the item index; the panel looks its name up.
    pub item: u16,
    /// TOTAL: `amount × count` for the quantity the stepper is showing.
    pub total: u32,
    /// HAVE: what the inventory holds right now.
    pub have: u32,
}

impl Ingredient {
    /// Whether this line is satisfied. Drawn red in the reference when it
    /// is not, and the CRAFT button is dead while any line is short.
    pub fn short(&self) -> bool {
        self.have < self.total
    }
}

/// The detail pane's ingredient table for `count` crafts.
///
/// Returns a fixed array plus a live count rather than a `Vec`: the table is
/// `MAX_RECIPE_INPUTS` wide by construction and this is called on every
/// stepper click.
pub fn ingredients(
    def: &RecipeDef,
    count: u16,
    inv: &[ItemStack; INV_SLOTS],
) -> ([Ingredient; MAX_RECIPE_INPUTS], usize) {
    let mut rows = [Ingredient {
        amount: 0,
        item: 0,
        total: 0,
        have: 0,
    }; MAX_RECIPE_INPUTS];
    let n = (def.n_inputs as usize).min(MAX_RECIPE_INPUTS);
    for (row, (item, units)) in rows.iter_mut().zip(def.inputs.iter()).take(n) {
        *row = Ingredient {
            amount: *units,
            item: *item,
            total: *units as u32 * count as u32,
            have: inv_count(inv, *item),
        };
    }
    (rows, n)
}

/// Seconds `count` crafts take, from the recipe's tick cost.
///
/// The bake keeps `ticks` exact and ≥ 1, so this is a display conversion and
/// never a rounding that the sim could disagree with — the sim counts ticks
/// and this counts the same ticks in seconds.
pub fn seconds(def: &RecipeDef, count: u16) -> f32 {
    def.ticks as f32 * count as f32 / TICK_HZ as f32
}

/// Seconds `count` crafts take at a bench `best` rungs strong — the highest
/// workbench in reach, `deploy::best_bench_in` over the client's mirror at
/// the sim's `STATION_RADIUS_M` (craft rebate v0). The sim's own
/// `craft::rebated_ticks`, so the pane quotes the time the queue will take
/// where the player is standing, not the recipe's list price.
pub fn seconds_at(def: &RecipeDef, count: u16, best: u8) -> f32 {
    sim_core::craft::rebated_ticks(def.ticks, def.station, best) as f32 * count as f32
        / TICK_HZ as f32
}

/// What a LOCKED recipe's detail pane says about where to learn it: the
/// bench tree it unlocks in and the node's price — the same `node_tier` the
/// sim demands at unlock and the same row the tree panel draws — or `None`
/// for a recipe that needs no blueprint or is already known. (The research
/// table's road needs a sample in hand and says so in the refusal instead.)
pub fn unlock_hint(
    rc: &sim_core::research::ResearchContent,
    known: u64,
    recipe: u16,
    def: &RecipeDef,
) -> Option<String> {
    if !def.blueprint || sim_core::research::knows(known, recipe) {
        return None;
    }
    let row = rc.row_for_recipe(recipe)?;
    Some(format!(
        "BLUEPRINT — UNLOCK AT WORKBENCH LEVEL {} · {} JUNK",
        sim_core::research::node_tier(def.station),
        row.cost
    ))
}

/// The station badge — the reference's yellow WORKBENCH LEVEL 1 REQUIRED.
/// `None` for a recipe with no station, which draws no badge at all rather
/// than a badge saying nothing is needed.
pub fn station_label(station: u8) -> Option<&'static str> {
    match station {
        STATION_NONE => None,
        STATION_WORKBENCH1 => Some("WORKBENCH LEVEL 1 REQUIRED"),
        STATION_WORKBENCH2 => Some("WORKBENCH LEVEL 2 REQUIRED"),
        STATION_WORKBENCH3 => Some("WORKBENCH LEVEL 3 REQUIRED"),
        STATION_FURNACE => Some("FURNACE REQUIRED"),
        _ => Some("STATION REQUIRED"),
    }
}

/// An item's display name, borrowed from the catalog, or `None` if no name
/// has arrived for it.
///
/// A catalog arrives in batches, so a missing name is a real state for the
/// first frames of a session rather than an error — which is why this
/// answers `None` instead of an empty string. `&[u8]` → `&str` can fail:
/// nothing on the wire promises UTF-8, and a name that is not gets treated
/// as absent rather than drawn as replacement characters.
pub fn item_name(catalog: &ItemCatalog, item: u16) -> Option<&str> {
    let raw = catalog.name(item as usize);
    if raw.is_empty() {
        return None;
    }
    std::str::from_utf8(raw).ok()
}

/// What a cell or a table row prints for an item: its name, or `#12` for an
/// index no name has arrived for. Drawing the index is honest; drawing an
/// empty cell is the dark-panel defect both this repo and elo's launcher
/// have a rule against.
///
/// Allocates, and that is the right trade here: it builds the string a
/// `Text` node is going to own anyway, and it runs when a panel is rebuilt
/// rather than per frame.
pub fn item_label(catalog: &ItemCatalog, item: u16) -> String {
    match item_name(catalog, item) {
        Some(n) => n.to_string(),
        None => format!("#{item}"),
    }
}

/// How many characters of 10 px bold condensed fit on one line of a 44 px
/// cell: 36 px inside the border and padding, ~5 px per glyph (`DECISIONS.md`
/// §open, `CELL_LINE_CHARS`).
pub const CELL_LINE_CHARS: usize = 7;

/// An item name cut to fit a 44 px cell, for the cells that have no icon to
/// be the picture (`NOW.md` §0p2 item 3).
///
/// The cell already clips (`Overflow::clip`), so nothing bleeds over the
/// border any more — but a clip cuts mid-glyph, and `Gunpowde` reads as a
/// defect where `Gunpow.` reads as an abbreviation. The rule, per word
/// because the cell is narrow and `Text` wraps on spaces: a word within
/// `max_chars` passes through untouched; a longer one is cut to the budget
/// with a trailing `.` marking the cut deliberate. `#12` labels and every
/// short name come back unchanged, which is why the common case allocates
/// only what `item_label` already allocated.
///
/// A budget of one has no room for the marker, so a cut word there is its
/// first glyph alone: a lone `.` says nothing, and `X.` — what this
/// emitted before the guard — is two characters on a one-character line,
/// breaking the very contract the function exists to hold. (Zero is not a
/// budget: a cell that cannot hold a glyph is not a cell, and zero
/// degrades to one rather than to silence.)
///
/// Pure and code-tier on purpose — the arithmetic is gated in
/// `tests/ui.rs`, where a Bevy system cannot be.
pub fn cell_abbrev(name: &str, max_chars: usize) -> String {
    let keep = max_chars.saturating_sub(1);
    name.split(' ')
        .map(|word| {
            if word.chars().count() <= max_chars {
                word.to_string()
            } else if keep == 0 {
                word.chars().take(1).collect()
            } else {
                let cut: String = word.chars().take(keep).collect();
                format!("{cut}.")
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// One live job in the queue strip. `eta_s` is the head job's remaining
/// time; the tail jobs have not started, so the reference draws them without
/// one and so does this.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Job {
    pub index: usize,
    pub recipe: u16,
    pub remaining: u16,
}

/// Seconds left on the head unit, from the core's `craft_eta_ticks`.
pub fn eta_seconds(eta_ticks: u16) -> f32 {
    eta_ticks as f32 / TICK_HZ as f32
}

/// The head unit's countdown, run on this side between `CraftQ`s.
///
/// The wire states the head's remaining ticks only when the queue changes
/// (`EventMsg::CraftQ`, "the client counts down locally between messages"),
/// and until this existed nothing counted: the queue strip printed the number
/// the last message carried until the next one replaced it, so a ten-second
/// craft read `10.0s` for ten seconds and then vanished. That frozen number
/// is most of why the craft screen felt laggy.
///
/// Clocked on the caller's seconds, not on ticks the sim agrees with: it is
/// a picture of a countdown the server owns, and the next `CraftQ` corrects
/// any drift by restating it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CraftClock {
    /// The head job as last heard, `(recipe, units remaining)`, so a new
    /// unit is told from a restated one.
    head: Option<(u8, u8)>,
    /// Seconds left on the head unit when it was heard.
    left_s: f32,
    /// The whole unit's seconds — the most it has been heard at, which is
    /// its full length when it was heard starting.
    unit_s: f32,
    /// When it was heard, on the caller's clock.
    at_s: f64,
}

impl CraftClock {
    /// A `CraftQ` landed: `head` is its first job (`None` for an empty
    /// queue), `eta_ticks` the head unit's remaining ticks, `now_s` the
    /// caller's clock.
    pub fn heard(&mut self, head: Option<(u8, u8)>, eta_ticks: u16, now_s: f64) {
        let left = eta_seconds(eta_ticks);
        if head != self.head {
            self.unit_s = left;
            self.head = head;
        } else {
            self.unit_s = self.unit_s.max(left);
        }
        self.left_s = left;
        self.at_s = now_s;
    }

    /// Seconds left on the head unit now, never below zero.
    pub fn left(&self, now_s: f64) -> f32 {
        if self.head.is_none() {
            return 0.0;
        }
        (self.left_s - (now_s - self.at_s).max(0.0) as f32).max(0.0)
    }

    /// How far through the head unit, 0..=1.
    pub fn progress(&self, now_s: f64) -> f32 {
        if self.head.is_none() || self.unit_s <= 0.0 {
            return 0.0;
        }
        (1.0 - self.left(now_s) / self.unit_s).clamp(0.0, 1.0)
    }
}

/// A countdown as the queue chip prints it: whole seconds, rounded up so it
/// reads `1s` until the unit lands — Rust's `24s` chip.
pub fn countdown_label(left_s: f32) -> String {
    format!("{}s", left_s.max(0.0).ceil() as u32)
}

#[cfg(test)]
mod clock_tests {
    use super::*;

    #[test]
    fn the_head_counts_down_between_messages() {
        let mut c = CraftClock::default();
        let ten = (10 * TICK_HZ) as u16;
        c.heard(Some((3, 2)), ten, 100.0);
        assert_eq!(c.left(100.0), 10.0);
        assert!((c.left(104.0) - 6.0).abs() < 1e-4);
        assert!((c.progress(104.0) - 0.4).abs() < 1e-4);
        // Past the end it holds at zero until the server restates it.
        assert_eq!(c.left(200.0), 0.0);
        assert_eq!(c.progress(200.0), 1.0);
    }

    #[test]
    fn a_new_unit_restarts_the_bar_and_a_tail_change_does_not() {
        let mut c = CraftClock::default();
        let ten = (10 * TICK_HZ) as u16;
        c.heard(Some((3, 2)), ten, 0.0);
        // Something was queued behind it at t=4: same head, 6 s left.
        c.heard(Some((3, 2)), (6 * TICK_HZ) as u16, 4.0);
        assert!(
            (c.progress(4.0) - 0.4).abs() < 1e-4,
            "the unit is still 10 s long"
        );
        // The unit finished and the next one started.
        c.heard(Some((3, 1)), ten, 10.0);
        assert_eq!(c.progress(10.0), 0.0);
        // An empty queue has nothing to count.
        c.heard(None, 0, 20.0);
        assert_eq!(c.left(20.0), 0.0);
        assert_eq!(c.progress(20.0), 0.0);
    }

    #[test]
    fn the_label_reads_like_rusts_chip() {
        assert_eq!(countdown_label(24.2), "25s");
        assert_eq!(countdown_label(10.0), "10s");
        assert_eq!(countdown_label(0.3), "1s");
        assert_eq!(countdown_label(0.0), "0s");
        assert_eq!(countdown_label(-2.0), "0s");
    }
}
