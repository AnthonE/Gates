//! The tech tree panel's **model** (tech tree v0) — every number the tree
//! screen needs, computed outside Bevy, `craft`'s posture exactly: pure
//! functions of `ClientCore`'s dripped tables plus the player's own mask
//! and pockets, gated headless in `tests/ui.rs` §H.
//!
//! The panel this feeds is the workbench's `E` (`interact::Verb::TechTree`,
//! the reference's tech-tree screen — `NOW.md` §0tt, spoken 2026-08-14).
//! What it shows is `content/research.toml`'s graph, **one bench tier per
//! tab** (operator, 2026-09-22: the bench's own tier, lower tiers as tabs,
//! higher tiers hidden): every researchable node of that tier, its coin
//! price, its parent, and which of four states it stands in for THIS
//! player right now. What it never does is decide — the unlock
//! request carries only the recipe index, and the parent, the bench tier
//! and the price are the sim's verdict (`research::unlock`). A node this
//! model calls `Ready` can still refuse on the server, and that refusal
//! arrives as a sentence (`refusals::RESEARCH`) exactly like every other.

use core::ops::RangeInclusive;

use sim_core::craft::CraftContent;
use sim_core::deploy::{best_bench_in, DeployContent, DeployRec};
use sim_core::gather::ItemStack;
use sim_core::limits::INV_SLOTS;
use sim_core::research::{
    knows, node_tier, ResearchContent, ResearchRow, NO_RECIPE, TABLE_RADIUS_M,
};

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
    /// The parent node's recipe, or `NO_RECIPE` for a root. It can name a
    /// row in another tab, which [`layout`] then draws as a root of this one
    /// (the state still reads Blocked until that parent is learned).
    pub requires: u16,
    /// The bench rung this node unlocks at (`research::node_tier` of the
    /// recipe's station — the same call the sim gates with, imported
    /// rather than re-derived so the drawn rung and the demanded rung
    /// cannot disagree).
    pub tier: u8,
    pub state: NodeState,
}

/// The tabs a tree opened at a bench of rung `bench` offers, lowest first:
/// that bench's own tier and every tier under it. **A higher tier's tree is
/// not drawn at all** — the reference's own shape, "each level of workbench
/// has its own tech tree" (Tech Tree Update, 2020-12-03) — and a higher bench
/// keeps the lower trees as tabs (operator, 2026-09-22), which is the sim's
/// ≥ (`Deploys::bench_near`) made visible rather than a second rule.
pub fn tabs(bench: u8) -> RangeInclusive<u8> {
    1..=bench.clamp(1, 3)
}

/// The icon stem the bench at the root of tier `tier`'s tree is drawn with —
/// the three bench items' own pictures (`ui::icons::STEMS`).
pub fn bench_glyph(tier: u8) -> &'static str {
    match tier {
        0 | 1 => "workbench",
        2 => "workbench_2",
        _ => "workbench_3",
    }
}

/// Whether a bench of rung ≥ `tier` still stands within reach of `pos`
/// (metres, the predictor's sim-truth position): `research::unlock`'s own
/// test — the same scan (`deploy::best_bench_in`) at the same radius
/// (`research::TABLE_RADIUS_M`) — so the panel closes exactly when the sim
/// would start refusing every button on it for want of a bench.
pub fn bench_in_reach(recs: &[DeployRec], dc: &DeployContent, pos: [f32; 3], tier: u8) -> bool {
    best_bench_in(recs, dc, pos[0], pos[2], TABLE_RADIUS_M) >= tier.max(1)
}

/// One row's state for this player: the sim's order of questions (known,
/// parent, coin), asked of the mask and the pockets.
fn state_of(row: &ResearchRow, known: u64, coin: u32) -> NodeState {
    if knows(known, row.recipe) {
        NodeState::Known
    } else if row.requires != NO_RECIPE && !knows(known, row.requires) {
        NodeState::Blocked
    } else if coin < row.cost as u32 {
        NodeState::Short
    } else {
        NodeState::Ready
    }
}

/// One tab of the tree as a list: every live research row whose bench rung
/// is `tier`, in table order (which is `research.toml`'s own order — the
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
    tier: u8,
    out: &mut Vec<Node>,
) {
    out.clear();
    let coin = sim_core::craft::inv_count(inv, rc.coin);
    for row in rc.rows[..rc.row_count as usize].iter() {
        if !row.is_live() || (row.recipe as usize) >= cc.recipes.len() {
            continue;
        }
        let row_tier = node_tier(cc.recipes[row.recipe as usize].station);
        if row_tier != tier {
            continue;
        }
        out.push(Node {
            recipe: row.recipe,
            item: row.item,
            cost: row.cost,
            requires: row.requires,
            tier: row_tier,
            state: state_of(row, known, coin),
        });
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
    /// Grid row = 1 + depth along the board's parents: row 0 is the bench
    /// every root hangs off, and a child is always exactly one row under its
    /// parent, which is what makes the edges drawable as one drop, one bus,
    /// one drop.
    pub row: u16,
}

/// One drawn edge, as indices into the `Placed` list — or, for a root's
/// edge, [`BENCH`] as the parent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Edge {
    pub parent: usize,
    pub child: usize,
}

/// The parent an edge from the bench carries. The bench is not a research
/// row, so it has no index in the placed list: it stands at row 0 in the
/// column [`layout`] returns, and every root of the tab hangs off it — the
/// reference's own picture, where each tree grows out of the workbench
/// that owns it.
pub const BENCH: usize = usize::MAX;

/// Lay tab `tier` out as the reference draws it: the bench alone on row 0,
/// each root's subtree below it as a block of columns as wide as its leaf
/// count, blocks side by side in table order, every node centred over its
/// children. Returns the bench's column; fills the placed nodes (table
/// order preserved among this tab's rows) and the edges, a root's edge
/// running from [`BENCH`].
///
/// **A row whose parent sits in another tab is placed as a root of this
/// one.** Its `requires` is untouched — the state still reads Blocked
/// until that parent is learned, because the sim still asks — but a line
/// to a node that is not on this board would be a line to nothing. Content
/// no longer ships such an edge (`validate::structural` refuses a
/// cross-tier `requires`); this is the panel not trusting that.
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
    tier: u8,
    placed: &mut Vec<Placed>,
    edges: &mut Vec<Edge>,
) -> u16 {
    placed.clear();
    edges.clear();
    let coin = sim_core::craft::inv_count(inv, rc.coin);
    let live: Vec<&ResearchRow> = rc.rows[..rc.row_count as usize]
        .iter()
        .filter(|r| {
            r.is_live()
                && (r.recipe as usize) < cc.recipes.len()
                && node_tier(cc.recipes[r.recipe as usize].station) == tier
        })
        .collect();
    let n = live.len();

    // The parent as THIS board sees it: the row's own `requires` when that
    // row is on the board, and no parent otherwise.
    let parent: Vec<u16> = live
        .iter()
        .map(|r| {
            if r.requires != NO_RECIPE && live.iter().any(|q| q.recipe == r.requires) {
                r.requires
            } else {
                NO_RECIPE
            }
        })
        .collect();

    // The subtree's column width: its leaf count, computed bottom-up by
    // walking rows in reverse table order — validate refuses a forward
    // reference nowhere, so a child CAN precede its parent in the file;
    // iterate to a fixed point instead, bounded by the row count.
    let mut width = vec![0u16; n];
    for _ in 0..=n {
        let mut moved = false;
        for i in 0..n {
            let kids: u16 = (0..n)
                .filter(|&k| parent[k] == live[i].recipe)
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

    // Depth = distance to a root along the board's own parents.
    let depth_of = |i: usize| -> u16 {
        let mut d = 0u16;
        let mut at = i;
        let mut steps = 0;
        while steps <= n && parent[at] != NO_RECIPE {
            let Some(p) = live.iter().position(|r| r.recipe == parent[at]) else {
                break;
            };
            at = p;
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
        if parent[i] == NO_RECIPE {
            stack.push((i, cursor));
            cursor += width[i];
        }
    }
    while let Some((i, c0)) = stack.pop() {
        col0[i] = c0;
        let mut child_c = c0;
        for k in (0..n).filter(|&k| parent[k] == live[i].recipe) {
            stack.push((k, child_c));
            child_c += width[k];
        }
    }

    for (i, r) in live.iter().enumerate() {
        placed.push(Placed {
            node: Node {
                recipe: r.recipe,
                item: r.item,
                cost: r.cost,
                requires: r.requires,
                tier,
                state: state_of(r, known, coin),
            },
            col: col0[i] + (width[i] - 1) / 2,
            // Row 0 is the bench's.
            row: depth_of(i) + 1,
        });
    }
    for (i, &par) in parent.iter().enumerate() {
        let from = if par == NO_RECIPE {
            BENCH
        } else {
            match live.iter().position(|q| q.recipe == par) {
                Some(p) => p,
                None => continue,
            }
        };
        edges.push(Edge {
            parent: from,
            child: i,
        });
    }
    // The bench stands over the middle of the whole board.
    cursor.max(1).saturating_sub(1) / 2
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
