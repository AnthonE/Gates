//! The tech tree screen (tech tree v0) — the workbench's `E`
//! (`ui::interact::Verb::TechTree`), drawn the way the reference draws its
//! bench tree (operator's frame, 2026-10-17): the whole screen. The board
//! takes everything left of a fixed right rail, with one LEVEL tab per tier
//! the bench reaches across its top (the bench's own tier open, lower tiers
//! a click away, higher tiers not drawn — operator, 2026-09-22). Learned
//! nodes are olive, the frontier is dark, everything past it wears a
//! padlock in its corner, the selected node is the reference's blue, and
//! light elbow lines run parent to child. The rail carries the junk count
//! and CLOSE on top, the selected item's picture, name, description and
//! recipe in the middle, and the price and a big UNLOCK at the foot.
//!
//! Every number is `ui::techtree`'s (pure, gated headless in `tests/ui.rs`
//! §M — the tidy-tree layout included); this file is nodes and handles.
//! Clicking a cell **selects** it; the unlock is the rail's button, offered
//! only on a `Ready` node — the reference's own two-step, and the reason a
//! mis-click on a locked cell costs nothing. The request carries the recipe
//! index alone; the parent, the tier and the price are the sim's verdict,
//! and its answer — learned, or the refusal's sentence — lands on the
//! status line ([`sync_status`]).

use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::prelude::*;

use super::{
    font, font_bold, spawn_tip, Hover, Panel, PanelRoot, Tip, Ui, BADGE, CELL_HOVER, CELL_SEL,
    LINE, PANEL_BG, SCRIM, SCROLL_PX_PER_LINE, STATUS, TEXT, TEXT_DIM, TEXT_SHORT,
};
use crate::render::feed::{Feed, Refused};
use crate::render::icons::{Icons, LOCK_TINT, PICTURE, PICTURE_DIM};
use crate::ui::craft::item_label;
use crate::ui::techtree::{self, NodeState, Placed, BENCH};
use client_core::core::ClientCore;

/// Grid metrics. The reference's cells are ~40 px at 720p over a tree a
/// hundred nodes wide; ours is a handful per tier, so the cells are drawn
/// bigger and the pitch wider, and the board still fits without a scroll.
const CELL_PX: f32 = 56.0;
const ICON_PX: f32 = 44.0;
const COL_W: f32 = 88.0;
const ROW_H: f32 = 84.0;
/// Connector width — a hairline, the reference's.
const EDGE_PX: f32 = 2.0;

/// The right rail's width and the height of the bar across the top of both
/// halves (the LEVEL tabs on the board, the junk count and CLOSE on the
/// rail).
const SIDE_W: f32 = 300.0;
const BAR_H: f32 = 38.0;

/// Over the HUD: the reference's tree is the whole screen, and the hotbar
/// drawn through the board's foot is not part of it. Under the tooltip (45)
/// and the death screen (`death::DEATH_Z`).
const TREE_Z: i32 = 40;

// ---- the board's paint, off the operator's frame -------------------------
/// A learned node — the reference's olive.
const NODE_KNOWN: Color = Color::srgba(0.333, 0.400, 0.200, 0.96);
const NODE_KNOWN_HOT: Color = Color::srgba(0.412, 0.494, 0.251, 0.98);
const NODE_KNOWN_RIM: Color = Color::srgba(0.494, 0.584, 0.310, 1.0);
/// The frontier: the parent is learned, this one is not. Dark, no lock.
const NODE_OPEN: Color = Color::srgba(0.110, 0.110, 0.098, 0.92);
const NODE_OPEN_HOT: Color = Color::srgba(0.200, 0.200, 0.184, 0.96);
const NODE_OPEN_RIM: Color = Color::srgba(0.560, 0.580, 0.500, 0.85);
/// Past the frontier: darker, a dimmer rim, and the padlock.
const NODE_SHUT: Color = Color::srgba(0.071, 0.071, 0.063, 0.88);
const NODE_SHUT_RIM: Color = Color::srgba(0.300, 0.300, 0.280, 0.75);
/// The selected node — the reference's blue, the hotbar's selection.
const NODE_SEL: Color = CELL_SEL;
const NODE_SEL_RIM: Color = Color::srgba(0.490, 0.690, 0.890, 1.0);
/// The connectors: light into a node you can reach, dim into one you cannot.
const EDGE_LIT: Color = Color::srgba(0.760, 0.780, 0.700, 0.80);
const EDGE_DIM: Color = Color::srgba(0.480, 0.490, 0.450, 0.55);

// ---- the bars and the button ---------------------------------------------
const BAR_BG: Color = Color::srgba(0.063, 0.063, 0.055, 0.94);
/// The open LEVEL tab, and the UNLOCK button — the reference's one green.
const GO: Color = Color::srgb(0.357, 0.475, 0.176);
const GO_HOT: Color = Color::srgb(0.427, 0.561, 0.220);
const GO_INK: Color = Color::srgb(0.835, 0.929, 0.651);
/// The button with nothing to buy behind it.
const GO_DEAD: Color = Color::srgba(0.180, 0.180, 0.169, 0.95);
/// The dark well the price sits in, beside the button.
const WELL: Color = Color::srgba(0.090, 0.090, 0.082, 0.95);
/// The path total — the orange the reference prints its tax figure in.
const PATH_INK: Color = Color::srgb(0.929, 0.553, 0.204);

/// A clickable node cell, carrying the recipe it selects.
#[derive(Component)]
pub struct NodeButton(pub u16);

/// The rail's unlock button.
#[derive(Component)]
pub struct UnlockGo(pub u16);

/// The rail's CLOSE.
#[derive(Component)]
pub struct TreeClose;

/// The scrolling board, so the wheel system can find and clamp it.
#[derive(Component)]
pub struct TreeScroll;

/// A LEVEL tab in the header, carrying the tier whose tree it shows.
#[derive(Component)]
pub struct TabButton(pub u8);

pub fn build_screen(commands: &mut Commands, ui: &Ui, core: &ClientCore, icons: &Icons) {
    let mut placed = Vec::new();
    let mut edges = Vec::new();
    let tab = ui.tech_tab.clamp(1, ui.tech_tier.max(1));
    let bench_col = techtree::layout(
        &core.research,
        &core.recipes,
        &core.inv,
        core.known(),
        tab,
        &mut placed,
        &mut edges,
    );
    let coin_have = sim_core::craft::inv_count(&core.inv, core.research.coin);

    commands
        .spawn((
            PanelRoot,
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Row,
                ..default()
            },
            BackgroundColor(SCRIM),
            GlobalZIndex(TREE_Z),
        ))
        .with_children(|root| {
            // Left: the LEVEL bar over the board.
            root.spawn(Node {
                flex_grow: 1.0,
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                ..default()
            })
            .with_children(|left| {
                tab_bar(left, ui, icons, tab);
                board(left, core, icons, ui, &placed, &edges, bench_col, tab);
                left.spawn((
                    Text::new(
                        "click a node to inspect it   ·   UNLOCK learns it   ·   Tab inventory   ·   Esc closes",
                    ),
                    font(11.0),
                    TextColor(TEXT_DIM),
                    Node {
                        align_self: AlignSelf::Center,
                        margin: UiRect::vertical(Val::Px(6.0)),
                        ..default()
                    },
                ));
            });
            sidebar(root, core, ui, icons, &placed, coin_have);
            spawn_tip(root);
        });
}

/// The bar across the board's top: one tab per tier this bench reaches,
/// lowest first, each with its bench's picture; the open one is green.
fn tab_bar(left: &mut ChildSpawnerCommands, ui: &Ui, icons: &Icons, tab: u8) {
    left.spawn((
        Node {
            width: Val::Percent(100.0),
            height: Val::Px(BAR_H),
            flex_shrink: 0.0,
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Stretch,
            ..default()
        },
        BackgroundColor(BAR_BG),
    ))
    .with_children(|bar| {
        for t in techtree::tabs(ui.tech_tier) {
            let open = t == tab;
            let rest = if open { GO } else { Color::NONE };
            bar.spawn((
                Button,
                TabButton(t),
                Node {
                    padding: UiRect::axes(Val::Px(16.0), Val::Px(0.0)),
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(8.0),
                    ..default()
                },
                BackgroundColor(rest),
                // The open tab is already lit; only the others light.
                Hover {
                    rest,
                    hot: if open { GO } else { CELL_HOVER },
                },
            ))
            .with_children(|b| {
                if let Some(handle) = icons.glyph(techtree::bench_glyph(t)) {
                    b.spawn((
                        ImageNode::new(handle),
                        Node {
                            width: Val::Px(24.0),
                            height: Val::Px(24.0),
                            ..default()
                        },
                        Pickable::IGNORE,
                    ));
                }
                b.spawn((
                    Text::new(format!("LEVEL {t}")),
                    font_bold(14.0),
                    TextColor(if open { GO_INK } else { TEXT_DIM }),
                    Pickable::IGNORE,
                ));
            });
        }
    });
}

/// The graph: an absolutely-positioned canvas centred in a scrolling frame.
/// Edges first so cells draw over them; each edge is a drop from the
/// parent, a bus at the midline, and a drop into the child — three thin
/// rects, the reference's own elbow.
#[allow(clippy::too_many_arguments)]
fn board(
    left: &mut ChildSpawnerCommands,
    core: &ClientCore,
    icons: &Icons,
    ui: &Ui,
    placed: &[Placed],
    edges: &[techtree::Edge],
    bench_col: u16,
    tab: u8,
) {
    let cols = placed
        .iter()
        .map(|p| p.col)
        .max()
        .map_or(1, |c| c + 1)
        .max(bench_col + 1);
    let rows = placed.iter().map(|p| p.row).max().map_or(1, |r| r + 1);
    let canvas_w = cols as f32 * COL_W;
    let canvas_h = (rows as f32 - 1.0) * ROW_H + CELL_PX;

    left.spawn((
        TreeScroll,
        Node {
            flex_grow: 1.0,
            flex_basis: Val::Px(0.0),
            min_height: Val::Px(0.0),
            flex_direction: FlexDirection::Column,
            padding: UiRect::axes(Val::Px(24.0), Val::Px(36.0)),
            overflow: Overflow::scroll(),
            ..default()
        },
    ))
    .with_children(|frame| {
        if placed.is_empty() {
            frame.spawn((
                Text::new(if core.research_have == 0 {
                    "nothing researchable has synced yet"
                } else {
                    "nothing to unlock at this level"
                }),
                font(14.0),
                TextColor(TEXT_DIM),
                Node {
                    align_self: AlignSelf::Center,
                    ..default()
                },
            ));
            return;
        }
        frame
            .spawn(Node {
                width: Val::Px(canvas_w),
                height: Val::Px(canvas_h),
                flex_shrink: 0.0,
                // Centred while it fits; flush left once it is wider than
                // the frame, so the scroll reaches its first column.
                margin: UiRect::horizontal(Val::Auto),
                ..default()
            })
            .with_children(|canvas| {
                let left_x = |col: u16| col as f32 * COL_W + (COL_W - CELL_PX) * 0.5;
                for e in edges {
                    // A root's edge starts at the bench, which is not in the
                    // placed list: it stands on row 0 at `bench_col`.
                    let (p_col, p_row) = if e.parent == BENCH {
                        (bench_col, 0)
                    } else {
                        (placed[e.parent].col, placed[e.parent].row)
                    };
                    let c = &placed[e.child];
                    let ink = if c.node.state == NodeState::Blocked {
                        EDGE_DIM
                    } else {
                        EDGE_LIT
                    };
                    let px = left_x(p_col) + (CELL_PX - EDGE_PX) * 0.5;
                    let cx = left_x(c.col) + (CELL_PX - EDGE_PX) * 0.5;
                    let top = p_row as f32 * ROW_H + CELL_PX;
                    let mid = c.row as f32 * ROW_H - (ROW_H - CELL_PX) * 0.5;
                    let bottom = c.row as f32 * ROW_H;
                    segment(canvas, px, top, EDGE_PX, mid - top, ink);
                    segment(
                        canvas,
                        px.min(cx),
                        mid,
                        (px - cx).abs() + EDGE_PX,
                        EDGE_PX,
                        ink,
                    );
                    segment(canvas, cx, mid, EDGE_PX, bottom - mid, ink);
                }
                bench_cell(canvas, icons, tab, left_x(bench_col));
                for p in placed {
                    cell(canvas, core, icons, ui, p, left_x(p.col));
                }
            });
    });
}

fn segment(canvas: &mut ChildSpawnerCommands, x: f32, y: f32, w: f32, h: f32, ink: Color) {
    canvas.spawn((
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(x),
            top: Val::Px(y),
            width: Val::Px(w),
            height: Val::Px(h),
            ..default()
        },
        BackgroundColor(ink),
        Pickable::IGNORE,
    ));
}

/// The root of the board: the bench the tab belongs to, drawn learned — it
/// is already yours, which is why you are standing at it.
fn bench_cell(canvas: &mut ChildSpawnerCommands, icons: &Icons, tier: u8, x: f32) {
    canvas
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(x),
                top: Val::Px(0.0),
                width: Val::Px(CELL_PX),
                height: Val::Px(CELL_PX),
                border: UiRect::all(Val::Px(1.0)),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
            BackgroundColor(NODE_KNOWN),
            BorderColor::all(NODE_KNOWN_RIM),
            Pickable::IGNORE,
        ))
        .with_children(|inner| {
            if let Some(handle) = icons.glyph(techtree::bench_glyph(tier)) {
                inner.spawn((
                    ImageNode::new(handle),
                    Node {
                        width: Val::Px(ICON_PX),
                        height: Val::Px(ICON_PX),
                        ..default()
                    },
                    Pickable::IGNORE,
                ));
            } else {
                inner.spawn((
                    Text::new(format!("WB{tier}")),
                    font_bold(12.0),
                    TextColor(TEXT),
                    Pickable::IGNORE,
                ));
            }
        });
}

/// A node's fill, its fill under the pointer, and its rim.
fn paint(state: NodeState, selected: bool) -> (Color, Color, Color) {
    if selected {
        return (NODE_SEL, NODE_SEL, NODE_SEL_RIM);
    }
    match state {
        NodeState::Known => (NODE_KNOWN, NODE_KNOWN_HOT, NODE_KNOWN_RIM),
        NodeState::Ready | NodeState::Short => (NODE_OPEN, NODE_OPEN_HOT, NODE_OPEN_RIM),
        NodeState::Blocked => (NODE_SHUT, NODE_OPEN_HOT, NODE_SHUT_RIM),
    }
}

/// The padlock in a cell's top-left corner — where the reference puts it,
/// so the picture under it still reads.
fn corner_lock(inner: &mut ChildSpawnerCommands, icons: &Icons, px: f32) {
    if let Some(lock) = icons.glyph("ui_lock") {
        inner.spawn((
            ImageNode::new(lock).with_color(LOCK_TINT),
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(2.0),
                top: Val::Px(2.0),
                width: Val::Px(px),
                height: Val::Px(px),
                ..default()
            },
            Pickable::IGNORE,
        ));
    }
}

/// One node cell: the item's picture on its state's fill, dimmed with a
/// corner padlock past the frontier.
fn cell(
    canvas: &mut ChildSpawnerCommands,
    core: &ClientCore,
    icons: &Icons,
    ui: &Ui,
    p: &Placed,
    x: f32,
) {
    let state = p.node.state;
    let blocked = state == NodeState::Blocked;
    let selected = ui.tech_sel == Some(p.node.recipe);
    let (rest, hot, rim) = paint(state, selected);
    let name = item_label(&core.catalog, p.node.item);

    canvas
        .spawn((
            Button,
            NodeButton(p.node.recipe),
            Tip(name.clone()),
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(x),
                top: Val::Px(p.row as f32 * ROW_H),
                width: Val::Px(CELL_PX),
                height: Val::Px(CELL_PX),
                border: UiRect::all(Val::Px(if selected { 2.0 } else { 1.0 })),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
            BackgroundColor(rest),
            Hover { rest, hot },
            BorderColor::all(rim),
        ))
        .with_children(|inner| {
            if let Some(handle) = icons.item(&name) {
                inner.spawn((
                    ImageNode::new(handle).with_color(if blocked { PICTURE_DIM } else { PICTURE }),
                    Node {
                        width: Val::Px(ICON_PX),
                        height: Val::Px(ICON_PX),
                        ..default()
                    },
                    Pickable::IGNORE,
                ));
            } else {
                // No icon baked for this item: the abbreviated name, dimmed
                // the same way — never an empty cell.
                inner.spawn((
                    Text::new(crate::ui::craft::cell_abbrev(
                        &name,
                        crate::ui::craft::CELL_LINE_CHARS,
                    )),
                    font(10.0),
                    TextColor(if blocked { TEXT_DIM } else { TEXT }),
                    Pickable::IGNORE,
                ));
            }
            if blocked {
                corner_lock(inner, icons, 16.0);
            }
        });
}

/// The right rail: the junk count and CLOSE, then what the selected node is,
/// then its price and the UNLOCK.
fn sidebar(
    root: &mut ChildSpawnerCommands,
    core: &ClientCore,
    ui: &Ui,
    icons: &Icons,
    placed: &[Placed],
    coin_have: u32,
) {
    let sel = ui
        .tech_sel
        .and_then(|r| placed.iter().find(|p| p.node.recipe == r))
        .map(|p| p.node);
    root.spawn((
        Node {
            width: Val::Px(SIDE_W),
            height: Val::Percent(100.0),
            flex_shrink: 0.0,
            flex_direction: FlexDirection::Column,
            border: UiRect::left(Val::Px(1.0)),
            ..default()
        },
        BackgroundColor(PANEL_BG),
        BorderColor::all(LINE),
    ))
    .with_children(|side| {
        top_bar(side, core, icons, coin_have);

        side.spawn(Node {
            flex_grow: 1.0,
            flex_direction: FlexDirection::Column,
            padding: UiRect::all(Val::Px(14.0)),
            row_gap: Val::Px(10.0),
            overflow: Overflow::clip(),
            ..default()
        })
        .with_children(|body| match sel {
            Some(node) => detail(body, core, icons, &node),
            None => {
                body.spawn((
                    Text::new("Select a node to see what it unlocks."),
                    font(13.0),
                    TextColor(TEXT_DIM),
                ));
                if !ui.status.is_empty() {
                    body.spawn((Text::new(ui.status.clone()), font(12.0), TextColor(STATUS)));
                }
            }
        });

        // The status line sits over the foot whichever node is up, so a
        // refusal is read beside the button that drew it.
        if sel.is_some() && !ui.status.is_empty() {
            side.spawn((
                Text::new(ui.status.clone()),
                font(12.0),
                TextColor(STATUS),
                Node {
                    margin: UiRect::horizontal(Val::Px(14.0)),
                    ..default()
                },
            ));
        }
        foot(side, core, icons, sel.as_ref(), coin_have);
    });
}

/// The rail's top: the junk in your pockets, and CLOSE.
fn top_bar(side: &mut ChildSpawnerCommands, core: &ClientCore, icons: &Icons, coin_have: u32) {
    side.spawn((
        Node {
            height: Val::Px(BAR_H),
            flex_shrink: 0.0,
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::SpaceBetween,
            padding: UiRect::left(Val::Px(12.0)),
            ..default()
        },
        BackgroundColor(BAR_BG),
    ))
    .with_children(|bar| {
        bar.spawn((
            Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: Val::Px(8.0),
                ..default()
            },
            Tip(item_label(&core.catalog, core.research.coin)),
            Interaction::default(),
        ))
        .with_children(|c| {
            coin_icon(c, core, icons, 22.0);
            c.spawn((
                Text::new(coin_have.to_string()),
                font_bold(16.0),
                TextColor(TEXT),
                Pickable::IGNORE,
            ));
        });
        bar.spawn((
            Button,
            TreeClose,
            Node {
                height: Val::Percent(100.0),
                padding: UiRect::axes(Val::Px(12.0), Val::Px(0.0)),
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: Val::Px(8.0),
                ..default()
            },
            BackgroundColor(Color::NONE),
            Hover::on(Color::NONE),
        ))
        .with_children(|b| {
            b.spawn((
                Node {
                    width: Val::Px(22.0),
                    height: Val::Px(22.0),
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::Center,
                    ..default()
                },
                BackgroundColor(Color::srgba(0.85, 0.84, 0.80, 0.92)),
                Pickable::IGNORE,
            ))
            .with_children(|x| {
                x.spawn((
                    Text::new("X"),
                    font_bold(14.0),
                    TextColor(Color::srgb(0.10, 0.10, 0.09)),
                    Pickable::IGNORE,
                ));
            });
            b.spawn((
                Text::new("CLOSE"),
                font_bold(14.0),
                TextColor(TEXT),
                Pickable::IGNORE,
            ));
        });
    });
}

/// The coin's own picture, or nothing where none was baked.
fn coin_icon(parent: &mut ChildSpawnerCommands, core: &ClientCore, icons: &Icons, px: f32) {
    if let Some(handle) = icons.item(&item_label(&core.catalog, core.research.coin)) {
        parent.spawn((
            ImageNode::new(handle),
            Node {
                width: Val::Px(px),
                height: Val::Px(px),
                ..default()
            },
            Pickable::IGNORE,
        ));
    }
}

/// What the selected node is: the picture and the name, its bench, what the
/// item is for, what one craft of it takes, and where it stands for you.
fn detail(
    body: &mut ChildSpawnerCommands,
    core: &ClientCore,
    icons: &Icons,
    node: &techtree::Node,
) {
    let name = item_label(&core.catalog, node.item);
    body.spawn(Node {
        flex_direction: FlexDirection::Row,
        align_items: AlignItems::Center,
        column_gap: Val::Px(12.0),
        ..default()
    })
    .with_children(|head| {
        head.spawn((
            Node {
                width: Val::Px(76.0),
                height: Val::Px(76.0),
                flex_shrink: 0.0,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.22)),
        ))
        .with_children(|tile| {
            if let Some(handle) = icons.item(&name) {
                tile.spawn((
                    ImageNode::new(handle),
                    Node {
                        width: Val::Px(66.0),
                        height: Val::Px(66.0),
                        ..default()
                    },
                ));
            }
        });
        head.spawn(Node {
            flex_direction: FlexDirection::Column,
            flex_shrink: 1.0,
            row_gap: Val::Px(4.0),
            ..default()
        })
        .with_children(|t| {
            t.spawn((Text::new(name.clone()), font_bold(19.0), TextColor(TEXT)));
            t.spawn((
                Text::new(techtree::tier_label(node.tier)),
                font_bold(11.0),
                TextColor(BADGE),
            ));
        });
    });

    // What it is for, in a line (`SUB_ITEM_DESC`).
    let desc = core.item_descs.get(node.item as usize);
    if !desc.is_empty() {
        body.spawn((
            Text::new(String::from_utf8_lossy(desc).into_owned()),
            font(13.0),
            TextColor(TEXT),
        ));
    }
    if let Some(def) = core.recipes.recipes.get(node.recipe as usize) {
        let line = techtree::craft_line(&core.catalog, def);
        if !line.is_empty() {
            body.spawn((Text::new(line), font(12.0), TextColor(TEXT_DIM)));
        }
    }

    match node.state {
        NodeState::Known => {
            body.spawn((
                Text::new("You know this blueprint — craft it from the crafting page."),
                font(12.0),
                TextColor(BADGE),
            ));
        }
        NodeState::Blocked => {
            // Name the node in the way — and its bench, since it can sit
            // in another tab — rather than pointing at "the node before".
            let before = core.research.row_for_recipe(node.requires).map(|r| {
                let tier = core
                    .recipes
                    .recipes
                    .get(r.recipe as usize)
                    .map_or(1, |d| sim_core::research::node_tier(d.station));
                format!(
                    "Unlock {} ({}) first.",
                    item_label(&core.catalog, r.item),
                    techtree::tier_label(tier)
                )
            });
            body.spawn((
                Text::new(before.unwrap_or_else(|| "Unlock the node before it first.".into())),
                font(12.0),
                TextColor(TEXT_DIM),
            ));
        }
        NodeState::Short => {
            body.spawn((
                Text::new("Not enough junk on you."),
                font(12.0),
                TextColor(TEXT_SHORT),
            ));
        }
        NodeState::Ready => {}
    }
}

/// The rail's foot: the path total in the row the reference spent on its
/// workbench tax (removed upstream, Nov 2025 — `reference/BLUEPRINTS.md`
/// §1), then the price in its well beside a full-width UNLOCK.
fn foot(
    side: &mut ChildSpawnerCommands,
    core: &ClientCore,
    icons: &Icons,
    sel: Option<&techtree::Node>,
    coin_have: u32,
) {
    side.spawn(Node {
        flex_shrink: 0.0,
        flex_direction: FlexDirection::Column,
        padding: UiRect::all(Val::Px(14.0)),
        row_gap: Val::Px(8.0),
        ..default()
    })
    .with_children(|f| {
        let total = sel.map_or(0, |n| {
            techtree::path_total(&core.research, core.known(), n.recipe)
        });
        if let Some(node) = sel.filter(|n| n.state != NodeState::Known) {
            f.spawn(Node {
                flex_direction: FlexDirection::Row,
                justify_content: JustifyContent::FlexEnd,
                align_items: AlignItems::Center,
                column_gap: Val::Px(8.0),
                ..default()
            })
            .with_children(|r| {
                r.spawn((
                    Text::new(total.to_string()),
                    font_bold(14.0),
                    TextColor(if total > coin_have {
                        TEXT_SHORT
                    } else {
                        PATH_INK
                    }),
                ));
                r.spawn((
                    Text::new(if total > node.cost as u32 {
                        "TOTAL REQUIRED (PATH)"
                    } else {
                        "TOTAL REQUIRED"
                    }),
                    font_bold(14.0),
                    TextColor(TEXT),
                ));
            });
        }

        f.spawn(Node {
            height: Val::Px(52.0),
            flex_direction: FlexDirection::Row,
            column_gap: Val::Px(6.0),
            ..default()
        })
        .with_children(|r| {
            // The price well.
            r.spawn((
                Node {
                    width: Val::Px(92.0),
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::Center,
                    column_gap: Val::Px(6.0),
                    ..default()
                },
                BackgroundColor(WELL),
            ))
            .with_children(|w| {
                coin_icon(w, core, icons, 26.0);
                let (cost, short) = match sel {
                    Some(n) if n.state != NodeState::Known => {
                        (n.cost.to_string(), coin_have < n.cost as u32)
                    }
                    _ => ("—".to_string(), false),
                };
                w.spawn((
                    Text::new(cost),
                    font_bold(18.0),
                    TextColor(if short { TEXT_SHORT } else { TEXT }),
                ));
            });

            // The button: live on a Ready node, a dead slab saying why on
            // anything else.
            let ready = sel.filter(|n| n.state == NodeState::Ready);
            let label = match sel.map(|n| n.state) {
                Some(s) => techtree::state_label(s),
                None => "UNLOCK",
            };
            let mut b = r.spawn((
                Node {
                    flex_grow: 1.0,
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::Center,
                    ..default()
                },
                BackgroundColor(if ready.is_some() { GO } else { GO_DEAD }),
            ));
            if let Some(node) = ready {
                b.insert((Button, UnlockGo(node.recipe), Hover::on(GO).hot(GO_HOT)));
            }
            b.with_children(|t| {
                t.spawn((
                    Text::new(label),
                    font_bold(20.0),
                    TextColor(if ready.is_some() { GO_INK } else { TEXT_DIM }),
                    Pickable::IGNORE,
                ));
            });
        });
    });
}

/// Clicks: a tab switches tree, a cell selects, the rail's button unlocks,
/// CLOSE closes. The mask and the board redraw on the `Known` restate that
/// follows — the bit is never set here, `client-core`'s own rule about
/// `SUB_KNOWN` being the authority (and `research::unlock` states it since
/// 2026-09-22, which it did not: the board never redrew on a purchase).
#[allow(clippy::type_complexity)]
pub fn clicks(
    mut ui: ResMut<Ui>,
    net: NonSend<super::super::Net>,
    cells: Query<(&Interaction, &NodeButton), Changed<Interaction>>,
    unlocks: Query<(&Interaction, &UnlockGo), Changed<Interaction>>,
    tabs: Query<(&Interaction, &TabButton), Changed<Interaction>>,
    close: Query<&Interaction, (Changed<Interaction>, With<TreeClose>)>,
) {
    if ui.panel != Panel::Tech {
        return;
    }
    if close.iter().any(|i| *i == Interaction::Pressed) {
        ui.panel = Panel::None;
        ui.tech_sel = None;
        ui.dirty = true;
        return;
    }
    for (interaction, tab) in tabs.iter() {
        if *interaction == Interaction::Pressed && ui.tech_tab != tab.0 {
            ui.tech_tab = tab.0;
            ui.tech_sel = None;
            ui.dirty = true;
        }
    }
    for (interaction, cell) in cells.iter() {
        if *interaction == Interaction::Pressed && ui.tech_sel != Some(cell.0) {
            ui.tech_sel = Some(cell.0);
            ui.dirty = true;
        }
    }
    for (interaction, go) in unlocks.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        let mut buf = [0u8; protocol::MAX_STREAM_MSG_BYTES];
        match protocol::encode_action_unlock(go.0, &mut buf) {
            Ok(len) => match net.session.send_action(&buf[..len]) {
                Ok(()) => ui.say("unlocking…".to_string()),
                Err(e) => ui.say(e.to_string()),
            },
            Err(e) => ui.say(format!("that unlock would not encode ({e:?})")),
        }
    }
}

/// The status line hears the sim: a blueprint learned, or the sentence for
/// why not. **Reads [`Feed`] and pops nothing** — `feed::drain` is the one
/// `pop_*` site in the client and `hud::feedback` is the other reader of the
/// same facts; this is a second `Res<Feed>`, which cannot consume anything.
pub fn sync_status(feed: Res<Feed>, mut ui: ResMut<Ui>, net: NonSend<super::super::Net>) {
    // The inventory hears it too since research table v1: a research table
    // is opened there, and a blueprint is read there with a right-click, so
    // "learned X" and "already known" belong on the line a player is
    // looking at when they happen.
    if !matches!(ui.panel, Panel::Tech | Panel::Inventory | Panel::Craft) {
        return;
    }
    let core = &net.session.core;
    for &(recipe, _coin) in feed.learned() {
        let item = core
            .recipes
            .recipes
            .get(recipe as usize)
            .map_or(sim_core::gather::NO_ITEM, |d| d.output);
        ui.say(format!("learned {}", item_label(&core.catalog, item)));
    }
    for (which, code, _item) in feed.refusals() {
        if which == Refused::Research {
            ui.say(crate::ui::refusals::research(code));
        }
    }
}

/// Wheel over the board — `craft::scroll`'s clamp on this panel's own
/// frame.
pub fn scroll(
    ui: Res<Ui>,
    mut wheel: MessageReader<MouseWheel>,
    mut cols: Query<(&mut ScrollPosition, &ComputedNode), With<TreeScroll>>,
) {
    if ui.panel != Panel::Tech {
        return;
    }
    let mut dy = 0.0;
    for ev in wheel.read() {
        dy += match ev.unit {
            MouseScrollUnit::Line => ev.y * SCROLL_PX_PER_LINE,
            MouseScrollUnit::Pixel => ev.y,
        };
    }
    if dy == 0.0 {
        return;
    }
    for (mut pos, computed) in cols.iter_mut() {
        let max = (computed.content_size().y - computed.size().y).max(0.0);
        pos.y = (pos.y - dy).clamp(0.0, max);
    }
}
