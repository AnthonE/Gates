//! The tech tree screen (tech tree v0) — the workbench's `E`
//! (`ui::interact::Verb::TechTree`), drawn the way the reference draws
//! its bench tree: a node **graph** over a scrim — the bench at the root,
//! icon cells with a padlock over everything unlearned, green connector
//! lines from parent to child, one LEVEL tab per tier the bench reaches
//! top-left (the bench's own tier open, lower tiers a click away, higher
//! tiers not drawn — operator, 2026-09-22), the coin count top-right — and
//! a detail sidebar on the selected node carrying the name, the price, and
//! the path total (`ui::techtree::path_total`, the number the reference
//! prints as TOTAL REQUIRED).
//!
//! Every number is `ui::techtree`'s (pure, gated headless in `tests/ui.rs`
//! §M — the tidy-tree layout included); this file is nodes and handles.
//! Clicking a cell **selects** it; the unlock is the sidebar's button,
//! offered only on a `Ready` node — the reference's own two-step, and the
//! reason a mis-click on a locked cell costs nothing. The request carries
//! the recipe index alone; the parent, the tier and the price are the
//! sim's verdict, and its answer — learned, or the refusal's sentence —
//! lands on the status line ([`sync_status`]).

use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::prelude::*;

use super::{
    font, font_bold, Panel, PanelRoot, Ui, BADGE, CELL_BG, CELL_FULL, LINE, LINE_HOT, PANEL_BG,
    SCRIM, SCROLL_PX_PER_LINE, TEXT, TEXT_DIM, TEXT_SHORT,
};
use crate::render::feed::{Feed, Refused};
use crate::ui::craft::item_label;
use crate::ui::techtree::{self, NodeState, Placed, BENCH};
use client_core::core::ClientCore;

/// Grid metrics: the craft browser's 44 px cell, spaced so an edge has
/// room to read as a line rather than a touch.
const CELL_PX: f32 = 44.0;
const COL_W: f32 = 60.0;
const ROW_H: f32 = 76.0;
/// Connector width — a hairline, the reference's.
const EDGE_PX: f32 = 2.0;
/// The drawn edge — the badge green, knocked back so cells stay the
/// brightest thing on the board.
const EDGE: Color = Color::srgba(0.604, 0.737, 0.361, 0.55);

/// The board and sidebar budgets: one 720p screen — title, board row,
/// hint — with the sidebar the reference's fixed right rail.
const BOARD_H: f32 = 430.0;
const BOARD_W: f32 = 430.0;
const SIDE_W: f32 = 210.0;

/// A clickable node cell, carrying the recipe it selects.
#[derive(Component)]
pub struct NodeButton(pub u16);

/// The sidebar's unlock button.
#[derive(Component)]
pub struct UnlockGo(pub u16);

/// The scrolling board, so the wheel system can find and clamp it.
#[derive(Component)]
pub struct TreeScroll;

/// A LEVEL tab in the header, carrying the tier whose tree it shows.
#[derive(Component)]
pub struct TabButton(pub u8);

pub fn build_screen(
    commands: &mut Commands,
    ui: &Ui,
    core: &ClientCore,
    icons: &super::super::icons::Icons,
) {
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
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                row_gap: Val::Px(8.0),
                ..default()
            },
            BackgroundColor(SCRIM),
        ))
        .with_children(|root| {
            // Header: LEVEL badge · title + status · coin count.
            root.spawn(Node {
                width: Val::Px(BOARD_W + SIDE_W + 8.0),
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: Val::Px(10.0),
                ..default()
            })
            .with_children(|h| {
                // One tab per tier this bench reaches, lowest first; the
                // open one wears the badge green.
                for t in techtree::tabs(ui.tech_tier) {
                    let open = t == tab;
                    h.spawn((
                        Button,
                        TabButton(t),
                        Node {
                            padding: UiRect::axes(Val::Px(8.0), Val::Px(3.0)),
                            border: UiRect::all(Val::Px(1.0)),
                            ..default()
                        },
                        BackgroundColor(if open { BADGE } else { CELL_BG }),
                        BorderColor::all(if open { BADGE } else { LINE }),
                    ))
                    .with_children(|b| {
                        b.spawn((
                            Text::new(format!("LEVEL {t}")),
                            font_bold(13.0),
                            TextColor(if open {
                                Color::srgb(0.08, 0.08, 0.06)
                            } else {
                                TEXT_DIM
                            }),
                            Pickable::IGNORE,
                        ));
                    });
                }
                h.spawn((Text::new("TECH TREE"), font_bold(22.0), TextColor(TEXT)));
                // The status line rides the header — a refusal sentence
                // replaces it until the next redraw.
                h.spawn((
                    Text::new(ui.status.clone()),
                    font(12.0),
                    TextColor(BADGE),
                    Node {
                        flex_grow: 1.0,
                        ..default()
                    },
                ));
                h.spawn((
                    Text::new(format!("{coin_have} junk")),
                    font_bold(15.0),
                    TextColor(TEXT),
                ));
            });

            // The board row: graph left, sidebar right.
            root.spawn(Node {
                flex_direction: FlexDirection::Row,
                column_gap: Val::Px(8.0),
                ..default()
            })
            .with_children(|row| {
                board(row, core, icons, ui, &placed, &edges, bench_col, tab);
                sidebar(row, core, ui, &placed);
            });

            root.spawn((
                Text::new(
                    "click a node to inspect it   -   LEVEL tabs switch tree   -   Tab opens inventory   -   Esc closes",
                ),
                font(12.0),
                TextColor(TEXT_DIM),
                Node {
                    margin: UiRect::top(Val::Px(6.0)),
                    ..default()
                },
            ));
        });
}

/// The graph: an absolutely-positioned canvas inside a scrolling frame.
/// Edges first so cells draw over them; each edge is a drop from the
/// parent, a bus at the midline, and a drop into the child — three thin
/// rects, the reference's own elbow.
#[allow(clippy::too_many_arguments)]
fn board(
    row: &mut ChildSpawnerCommands,
    core: &ClientCore,
    icons: &super::super::icons::Icons,
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
    let canvas_w = (cols as f32 * COL_W).max(BOARD_W - 20.0);
    let canvas_h = (rows as f32 * ROW_H).max(1.0);

    row.spawn((
        TreeScroll,
        Node {
            width: Val::Px(BOARD_W),
            height: Val::Px(BOARD_H),
            padding: UiRect::all(Val::Px(10.0)),
            border: UiRect::all(Val::Px(1.0)),
            overflow: Overflow::scroll_y(),
            ..default()
        },
        BackgroundColor(PANEL_BG),
        BorderColor::all(LINE),
    ))
    .with_children(|frame| {
        frame
            .spawn(Node {
                width: Val::Px(canvas_w),
                height: Val::Px(canvas_h),
                ..default()
            })
            .with_children(|canvas| {
                if placed.is_empty() {
                    canvas.spawn((
                        Text::new(if core.research_have == 0 {
                            "nothing researchable has synced yet"
                        } else {
                            "nothing to unlock at this level"
                        }),
                        font(13.0),
                        TextColor(TEXT_DIM),
                    ));
                    return;
                }
                let center_x = |col: u16| col as f32 * COL_W + (COL_W - CELL_PX) * 0.5;
                for e in edges {
                    // A root's edge starts at the bench, which is not in the
                    // placed list: it stands on row 0 at `bench_col`.
                    let (p_col, p_row) = if e.parent == BENCH {
                        (bench_col, 0)
                    } else {
                        (placed[e.parent].col, placed[e.parent].row)
                    };
                    let c = &placed[e.child];
                    let px = center_x(p_col) + CELL_PX * 0.5;
                    let cx = center_x(c.col) + CELL_PX * 0.5;
                    let top = p_row as f32 * ROW_H + CELL_PX;
                    let mid = c.row as f32 * ROW_H - (ROW_H - CELL_PX) * 0.5;
                    let bottom = c.row as f32 * ROW_H;
                    segment(canvas, px, top, EDGE_PX, mid - top);
                    segment(canvas, px.min(cx), mid, (px - cx).abs() + EDGE_PX, EDGE_PX);
                    segment(canvas, cx, mid, EDGE_PX, bottom - mid);
                }
                bench_cell(canvas, icons, tab, center_x(bench_col));
                for p in placed {
                    cell(canvas, core, icons, ui, p, center_x(p.col));
                }
            });
    });
}

fn segment(canvas: &mut ChildSpawnerCommands, x: f32, y: f32, w: f32, h: f32) {
    canvas.spawn((
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(x),
            top: Val::Px(y),
            width: Val::Px(w),
            height: Val::Px(h),
            ..default()
        },
        BackgroundColor(EDGE),
        Pickable::IGNORE,
    ));
}

/// The root of the board: the bench the tab belongs to, drawn lit — it is
/// already yours, which is why you are standing at it.
fn bench_cell(
    canvas: &mut ChildSpawnerCommands,
    icons: &super::super::icons::Icons,
    tier: u8,
    x: f32,
) {
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
            BackgroundColor(CELL_FULL),
            BorderColor::all(BADGE),
            Pickable::IGNORE,
        ))
        .with_children(|inner| {
            if let Some(handle) = icons.glyph(techtree::bench_glyph(tier)) {
                inner.spawn((
                    ImageNode::new(handle),
                    Node {
                        width: Val::Px(34.0),
                        height: Val::Px(34.0),
                        ..default()
                    },
                    Pickable::IGNORE,
                ));
            } else {
                inner.spawn((
                    Text::new(format!("WB{tier}")),
                    font_bold(11.0),
                    TextColor(TEXT),
                    Pickable::IGNORE,
                ));
            }
        });
}

/// One node cell: the item's icon, dimmed with a padlock over it until it
/// is learned — the reference's read, where the lock is the *unresearched*
/// mark and the highlight border is what says "buyable now".
fn cell(
    canvas: &mut ChildSpawnerCommands,
    core: &ClientCore,
    icons: &super::super::icons::Icons,
    ui: &Ui,
    p: &Placed,
    x: f32,
) {
    let known = p.node.state == NodeState::Known;
    let ready = p.node.state == NodeState::Ready;
    let selected = ui.tech_sel == Some(p.node.recipe);
    let name = item_label(&core.catalog, p.node.item);

    let mut c = canvas.spawn((
        Button,
        NodeButton(p.node.recipe),
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(x),
            top: Val::Px(p.row as f32 * ROW_H),
            width: Val::Px(CELL_PX),
            height: Val::Px(CELL_PX),
            border: UiRect::all(Val::Px(1.0)),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            ..default()
        },
        BackgroundColor(if known { CELL_FULL } else { CELL_BG }),
        BorderColor::all(if selected {
            LINE_HOT
        } else if ready {
            BADGE
        } else {
            LINE
        }),
    ));
    c.with_children(|inner| {
        if let Some(handle) = icons.item(&name) {
            inner.spawn((
                ImageNode {
                    color: if known {
                        Color::WHITE
                    } else {
                        Color::srgba(1.0, 1.0, 1.0, 0.35)
                    },
                    ..ImageNode::new(handle.clone())
                },
                Node {
                    width: Val::Px(34.0),
                    height: Val::Px(34.0),
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
                font(9.0),
                TextColor(if known { TEXT } else { TEXT_DIM }),
                Pickable::IGNORE,
            ));
        }
        if !known {
            if let Some(lock) = icons.glyph("code_lock") {
                inner.spawn((
                    ImageNode::new(lock.clone()),
                    Node {
                        position_type: PositionType::Absolute,
                        width: Val::Px(18.0),
                        height: Val::Px(18.0),
                        ..default()
                    },
                    Pickable::IGNORE,
                ));
            }
        }
    });
}

/// The right rail: what the selected node is, what it costs, what the
/// whole path still costs, and the unlock button when it is buyable.
fn sidebar(row: &mut ChildSpawnerCommands, core: &ClientCore, ui: &Ui, placed: &[Placed]) {
    row.spawn((
        Node {
            width: Val::Px(SIDE_W),
            height: Val::Px(BOARD_H),
            flex_direction: FlexDirection::Column,
            padding: UiRect::all(Val::Px(10.0)),
            row_gap: Val::Px(6.0),
            border: UiRect::all(Val::Px(1.0)),
            ..default()
        },
        BackgroundColor(PANEL_BG),
        BorderColor::all(LINE),
    ))
    .with_children(|side| {
        let Some(sel) = ui
            .tech_sel
            .and_then(|r| placed.iter().find(|p| p.node.recipe == r))
        else {
            side.spawn((Text::new("select a node"), font(13.0), TextColor(TEXT_DIM)));
            return;
        };
        let node = &sel.node;
        side.spawn((
            Text::new(item_label(&core.catalog, node.item)),
            font_bold(16.0),
            TextColor(TEXT),
        ));
        side.spawn((
            Text::new(techtree::tier_label(node.tier)),
            font(11.0),
            TextColor(BADGE),
        ));
        side.spawn((
            Text::new(format!("{} junk", node.cost)),
            font(13.0),
            TextColor(if node.state == NodeState::Short {
                TEXT_SHORT
            } else {
                TEXT
            }),
        ));
        let total = techtree::path_total(&core.research, core.known(), node.recipe);
        if node.state != NodeState::Known {
            side.spawn((
                Text::new(format!("TOTAL REQUIRED   {total} junk")),
                font(12.0),
                TextColor(TEXT_DIM),
            ));
        }
        match node.state {
            NodeState::Known => {
                side.spawn((Text::new("KNOWN"), font_bold(13.0), TextColor(TEXT_DIM)));
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
                        "LOCKED — unlock {} ({}) first",
                        item_label(&core.catalog, r.item),
                        techtree::tier_label(tier)
                    )
                });
                side.spawn((
                    Text::new(
                        before.unwrap_or_else(|| "LOCKED — unlock the node before it first".into()),
                    ),
                    font(12.0),
                    TextColor(TEXT_DIM),
                ));
            }
            NodeState::Short => {
                side.spawn((
                    Text::new("NEED JUNK"),
                    font_bold(13.0),
                    TextColor(TEXT_SHORT),
                ));
            }
            NodeState::Ready => {
                side.spawn((
                    Button,
                    UnlockGo(node.recipe),
                    Node {
                        margin: UiRect::top(Val::Px(8.0)),
                        padding: UiRect::axes(Val::Px(14.0), Val::Px(7.0)),
                        border: UiRect::all(Val::Px(1.0)),
                        justify_content: JustifyContent::Center,
                        ..default()
                    },
                    BackgroundColor(CELL_FULL),
                    BorderColor::all(LINE_HOT),
                ))
                .with_children(|b| {
                    b.spawn((
                        Text::new(format!("UNLOCK · {} JUNK", node.cost)),
                        font_bold(13.0),
                        TextColor(LINE_HOT),
                        Pickable::IGNORE,
                    ));
                });
            }
        }
    });
}

/// Clicks: a tab switches tree, a cell selects, the sidebar's button
/// unlocks. The mask and the board redraw on the `Known` restate that
/// follows — the bit is never set here, `client-core`'s own rule about
/// `SUB_KNOWN` being the authority (and `research::unlock` states it since
/// 2026-09-22, which it did not: the board never redrew on a purchase).
pub fn clicks(
    mut ui: ResMut<Ui>,
    net: NonSend<super::super::Net>,
    cells: Query<(&Interaction, &NodeButton), Changed<Interaction>>,
    unlocks: Query<(&Interaction, &UnlockGo), Changed<Interaction>>,
    tabs: Query<(&Interaction, &TabButton), Changed<Interaction>>,
) {
    if ui.panel != Panel::Tech {
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
    if ui.panel != Panel::Tech && ui.panel != Panel::Inventory {
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
