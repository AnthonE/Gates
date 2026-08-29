//! The tech tree panel's **model** (tech tree v0) — every number the tree
//! screen needs, computed outside Bevy, `craft`'s posture exactly: pure
//! functions of `ClientCore`'s dripped tables plus the player's own mask
//! and pockets, gated headless in `tests/ui.rs` §H.
//!
//! The panel this feeds is the workbench's `E` (`interact::Verb::TechTree`,
//! the reference's tech-tree screen — `NOW.md` §0tt, spoken 2026-08-14).
//! What it shows is `content/research.toml`'s graph: every researchable
//! node, its coin price, its parent, and which of four states it stands in
//! for THIS player right now. What it never does is decide — the unlock
//! request carries only the recipe index, and the parent, the bench tier
//! and the price are the sim's verdict (`research::unlock`). A node this
//! model calls `Ready` can still refuse on the server, and that refusal
//! arrives as a sentence (`refusals::RESEARCH`) exactly like every other.

use sim_core::craft::CraftContent;
use sim_core::gather::ItemStack;
use sim_core::limits::INV_SLOTS;
use sim_core::research::{knows, node_tier, ResearchContent, NO_RECIPE};

/// What a node is to this player, in the order the panel colours them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeState {
    /// Already learned — through either verb; the mask does not remember
    /// which, and the panel has no reason to care.
    Known,
    /// Parent learned (or a root), coin in pocket: one click from yours.
    Ready,
    /// Parent learned, coin short. Drawn hot rather than grey — the
    /// difference between "go earn" and "go unlock" is the difference
    /// between a farming trip and a tree decision, and the colour is how
    /// the panel says which.
    Short,
    /// Parent not yet learned. The tree's own gate.
    Blocked,
}

/// One drawable node.
#[derive(Clone, Copy, Debug)]
pub struct Node {
    /// The recipe the unlock names on the wire (`encode_action_unlock`).
    pub recipe: u16,
    /// The item the recipe outputs — what the panel draws an icon and a
    /// name for.
    pub item: u16,
    /// The coin price of this node alone.
    pub cost: u16,
    /// The parent node's recipe, or `NO_RECIPE` for a root — the edge the
    /// panel draws an indent (and one day a line) from.
    pub requires: u16,
    /// The bench rung this node unlocks at (`research::node_tier` of the
    /// recipe's station — the same call the sim gates with, imported
    /// rather than re-derived so the drawn rung and the demanded rung
    /// cannot disagree).
    pub tier: u8,
    pub state: NodeState,
}

/// The tree as drawn: every live research row, grouped by tier ascending,
/// table order within a tier (which is `research.toml`'s own order — the
/// file is written root-before-child within each path, and keeping its
/// order costs nothing while a topological sort would invent one).
///
/// Takes `&mut Vec` for the reason `craft::rows` does: the panel calls
/// this on every rebuild and a keystroke must not allocate.
pub fn rows(
    rc: &ResearchContent,
    cc: &CraftContent,
    inv: &[ItemStack; INV_SLOTS],
    known: u64,
    out: &mut Vec<Node>,
) {
    out.clear();
    let coin = sim_core::craft::inv_count(inv, rc.coin);
    for want_tier in 1..=3u8 {
        for row in rc.rows[..rc.row_count as usize].iter() {
            if !row.is_live() || (row.recipe as usize) >= cc.recipes.len() {
                continue;
            }
            let tier = node_tier(cc.recipes[row.recipe as usize].station);
            if tier != want_tier {
                continue;
            }
            let state = if knows(known, row.recipe) {
                NodeState::Known
            } else if row.requires != NO_RECIPE && !knows(known, row.requires) {
                NodeState::Blocked
            } else if coin < row.cost as u32 {
                NodeState::Short
            } else {
                NodeState::Ready
            };
            out.push(Node {
                recipe: row.recipe,
                item: row.item,
                cost: row.cost,
                requires: row.requires,
                tier,
                state,
            });
        }
    }
}

/// What the tree still charges to reach `recipe` from here: its own cost
/// plus every unlearned ancestor's, following `requires` up. The number
/// the reference prints on an item page as the "tech tree path total" —
/// and 0 for a node already known.
///
/// The walk is bounded by the row count, the validator's own cycle
/// budget: content with a cycle never bakes (`validate.rs`), so the bound
/// is a belt on braces, not a live case.
pub fn path_total(rc: &ResearchContent, known: u64, recipe: u16) -> u32 {
    let mut total = 0u32;
    let mut at = recipe;
    let mut steps = 0usize;
    while at != NO_RECIPE && steps <= rc.row_count as usize {
        if knows(known, at) {
            break;
        }
        let Some(row) = rc.row_for_recipe(at) else {
            break;
        };
        total += row.cost as u32;
        at = row.requires;
        steps += 1;
    }
    total
}

/// One node placed on the tree's grid (the reference's own layout: a
/// forest of columns, parents above children, drawn edges between).
#[derive(Clone, Copy, Debug)]
pub struct Placed {
    pub node: Node,
    /// Grid column. Roots sit over their subtrees; a parent is centred
    /// on its children's span.
    pub col: u16,
    /// Grid row = depth along `requires` — a child is always exactly one
    /// row under its parent, which is what makes the edges drawable as
    /// one drop, one bus, one drop.
    pub row: u16,
}

/// One drawn edge, as indices into the `Placed` list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Edge {
    pub parent: usize,
    pub child: usize,
}

/// Lay the tree out as the reference draws it: each root's subtree is a
/// block of columns as wide as its leaf count, blocks side by side in
/// table order, every node centred over its children. Returns the placed
/// nodes (table order preserved — index n is row n of the research
/// table's live set) and the parent→child edges.
///
/// Pure and recursion-free: the subtree walk is a hand stack bounded by
/// the row count, because content with a cycle never bakes
/// (`validate.rs`) and a stack is cheaper to reason about than a
/// recursion depth.
pub fn layout(
    rc: &ResearchContent,
    cc: &CraftContent,
    inv: &[ItemStack; INV_SLOTS],
    known: u64,
    placed: &mut Vec<Placed>,
    edges: &mut Vec<Edge>,
) {
    placed.clear();
    edges.clear();
    let coin = sim_core::craft::inv_count(inv, rc.coin);
    let live: Vec<&sim_core::research::ResearchRow> = rc.rows[..rc.row_count as usize]
        .iter()
        .filter(|r| r.is_live() && (r.recipe as usize) < cc.recipes.len())
        .collect();

    // The subtree's column width: its leaf count, computed bottom-up by
    // walking rows in reverse table order — validate refuses a forward
    // reference nowhere, so a child CAN precede its parent in the file;
    // iterate to a fixed point instead, bounded by the row count.
    let n = live.len();
    let mut width = vec![0u16; n];
    for _ in 0..=n {
        let mut moved = false;
        for i in 0..n {
            let kids: u16 = (0..n)
                .filter(|&k| live[k].requires == live[i].recipe)
                .map(|k| width[k])
                .sum();
            let w = kids.max(1);
            if width[i] != w {
                width[i] = w;
                moved = true;
            }
        }
        if !moved {
            break;
        }
    }

    // Depth = distance to a root along `requires`.
    let depth_of = |mut recipe: u16| -> u16 {
        let mut d = 0u16;
        let mut steps = 0;
        while steps <= n {
            let Some(i) = live.iter().position(|r| r.recipe == recipe) else {
                break;
            };
            if live[i].requires == NO_RECIPE {
                break;
            }
            recipe = live[i].requires;
            d += 1;
            steps += 1;
        }
        d
    };

    // Place: roots left to right in table order, each subtree's children
    // packed left to right inside the parent's span.
    let mut col0 = vec![0u16; n];
    let mut cursor = 0u16;
    // (index, span start). Every entry carries its own span, computed at
    // push time in table order — so the pop order is bookkeeping and the
    // columns are a function of the table alone.
    let mut stack: Vec<(usize, u16)> = Vec::new();
    for i in 0..n {
        if live[i].requires == NO_RECIPE {
            stack.push((i, cursor));
            cursor += width[i];
        }
    }
    while let Some((i, c0)) = stack.pop() {
        col0[i] = c0;
        let mut child_c = c0;
        for k in (0..n).filter(|&k| live[k].requires == live[i].recipe) {
            stack.push((k, child_c));
            child_c += width[k];
        }
    }

    for (i, r) in live.iter().enumerate() {
        let state = if knows(known, r.recipe) {
            NodeState::Known
        } else if r.requires != NO_RECIPE && !knows(known, r.requires) {
            NodeState::Blocked
        } else if coin < r.cost as u32 {
            NodeState::Short
        } else {
            NodeState::Ready
        };
        placed.push(Placed {
            node: Node {
                recipe: r.recipe,
                item: r.item,
                cost: r.cost,
                requires: r.requires,
                tier: node_tier(cc.recipes[r.recipe as usize].station),
                state,
            },
            col: col0[i] + (width[i] - 1) / 2,
            row: depth_of(r.recipe),
        });
    }
    for (i, r) in live.iter().enumerate() {
        if r.requires == NO_RECIPE {
            continue;
        }
        if let Some(p) = live.iter().position(|q| q.recipe == r.requires) {
            edges.push(Edge {
                parent: p,
                child: i,
            });
        }
    }
}

/// The tier headline over a group — the craft badge's phrasing, so the
/// two screens name a bench one way.
pub fn tier_label(tier: u8) -> &'static str {
    match tier {
        1 => "WORKBENCH LEVEL 1",
        2 => "WORKBENCH LEVEL 2",
        3 => "WORKBENCH LEVEL 3",
        _ => "WORKBENCH",
    }
}

/// The state word the row draws where the craft panel draws LOCKED.
pub fn state_label(state: NodeState) -> &'static str {
    match state {
        NodeState::Known => "KNOWN",
        NodeState::Ready => "UNLOCK",
        NodeState::Short => "NEED JUNK",
        NodeState::Blocked => "LOCKED",
    }
}
