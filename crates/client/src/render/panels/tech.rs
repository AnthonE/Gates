//! The tech tree screen (tech tree v0) — the workbench's `E`
//! (`ui::interact::Verb::TechTree`), the reference's bench tree read
//! through this repo's panel idiom: every number from `ui::techtree`
//! (pure, gated headless in `tests/ui.rs` §H), nodes and handles here and
//! nothing else.
//!
//! The screen is one scrolling column: a tier headline per bench rung,
//! then that rung's nodes as rows — icon, name, the `requires` parent as
//! an indented line, the coin price, and a state word. A `Ready` row is a
//! button; everything else says why it is not
//! (`ui::techtree::state_label`). The unlock request carries the recipe
//! index alone, and the sim's refusal — a parent raced learned, a bench
//! demolished under the open panel — comes back as a sentence on the
//! status line like every other verb's.

use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::prelude::*;

use super::{
    font, font_bold, Panel, PanelRoot, Ui, BADGE, CELL_BG, CELL_FULL, LINE, LINE_HOT, PANEL_BG,
    SCRIM, SCROLL_PX_PER_LINE, TEXT, TEXT_DIM, TEXT_SHORT,
};
use crate::ui::craft::item_label;
use crate::ui::techtree::{self, Node as TreeNode, NodeState};
use client_core::core::ClientCore;

/// The tree column's height budget: `PANEL_H`'s derivation, one screen —
/// title + status + column + hint inside 720 px.
const TREE_H: f32 = 430.0;
const TREE_W: f32 = 460.0;

/// A clickable node row, carrying the recipe its unlock names.
#[derive(Component)]
pub struct NodeButton(pub u16);

/// The scrolling column, so the wheel system can find and clamp it.
#[derive(Component)]
pub struct TreeScroll;

pub fn build_screen(
    commands: &mut Commands,
    ui: &Ui,
    core: &ClientCore,
    icons: &super::super::icons::Icons,
) {
    let mut nodes = Vec::new();
    techtree::rows(
        &core.research,
        &core.recipes,
        &core.inv,
        core.known(),
        &mut nodes,
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
            root.spawn((Text::new("TECH TREE"), font_bold(26.0), TextColor(TEXT)));
            // The status line, always drawn — `inv::header`'s rule: the
            // panel needs somewhere to say why something did not happen.
            root.spawn((
                Text::new(if ui.status.is_empty() {
                    format!("{coin_have} obol")
                } else {
                    ui.status.clone()
                }),
                font(13.0),
                TextColor(BADGE),
            ));

            // The tree itself: a bordered, scrolling column.
            root.spawn((
                TreeScroll,
                Node {
                    width: Val::Px(TREE_W),
                    height: Val::Px(TREE_H),
                    flex_direction: FlexDirection::Column,
                    padding: UiRect::all(Val::Px(10.0)),
                    row_gap: Val::Px(4.0),
                    border: UiRect::all(Val::Px(1.0)),
                    overflow: Overflow::scroll_y(),
                    ..default()
                },
                BackgroundColor(PANEL_BG),
                BorderColor::all(LINE),
            ))
            .with_children(|col| {
                let mut drawn_tier = 0u8;
                if nodes.is_empty() {
                    // The drip has not landed yet, or the shard's content
                    // gates nothing. Said in words — an empty dark panel
                    // is the defect the status-line rule exists for.
                    col.spawn((
                        Text::new("nothing researchable has synced yet"),
                        font(13.0),
                        TextColor(TEXT_DIM),
                    ));
                }
                for node in &nodes {
                    if node.tier != drawn_tier {
                        drawn_tier = node.tier;
                        col.spawn((
                            Text::new(techtree::tier_label(node.tier)),
                            font_bold(14.0),
                            TextColor(BADGE),
                            Node {
                                margin: UiRect::top(Val::Px(if node.tier == 1 {
                                    0.0
                                } else {
                                    8.0
                                })),
                                ..default()
                            },
                        ));
                    }
                    node_row(col, core, icons, node);
                }
            });

            root.spawn((
                Text::new("click an UNLOCK to learn it   -   Tab opens inventory   -   Esc closes"),
                font(12.0),
                TextColor(TEXT_DIM),
                Node {
                    margin: UiRect::top(Val::Px(6.0)),
                    ..default()
                },
            ));
        });
}

/// One node row. A `Ready` row gets the button component and the hot
/// border; the rest draw flat — `Interaction` on a row that could only
/// refuse would promise a click the panel knows is dead.
fn node_row(
    col: &mut ChildSpawnerCommands,
    core: &ClientCore,
    icons: &super::super::icons::Icons,
    node: &TreeNode,
) {
    let name = item_label(&core.catalog, node.item);
    let ready = node.state == NodeState::Ready;
    let known = node.state == NodeState::Known;

    let mut row = col.spawn((
        Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: Val::Px(8.0),
            padding: UiRect::all(Val::Px(6.0)),
            border: UiRect::all(Val::Px(1.0)),
            // The tree's one drawn edge: a child indents under its
            // parent, which for a three-deep table reads as the graph
            // without a line renderer.
            margin: UiRect::left(Val::Px(if node.requires == sim_core::research::NO_RECIPE {
                0.0
            } else {
                18.0
            })),
            ..default()
        },
        BackgroundColor(if known { CELL_FULL } else { CELL_BG }),
        BorderColor::all(if ready { LINE_HOT } else { LINE }),
    ));
    if ready {
        row.insert((Button, NodeButton(node.recipe)));
    }
    row.with_children(|r| {
        if let Some(handle) = icons.item(&name) {
            r.spawn((
                ImageNode::new(handle.clone()),
                Node {
                    width: Val::Px(22.0),
                    height: Val::Px(22.0),
                    ..default()
                },
                Pickable::IGNORE,
            ));
        }
        r.spawn((
            Text::new(name),
            font(14.0),
            TextColor(TEXT),
            Node {
                width: Val::Px(190.0),
                ..default()
            },
            Pickable::IGNORE,
        ));
        r.spawn((
            Text::new(format!("{} obol", node.cost)),
            font(13.0),
            TextColor(if node.state == NodeState::Short {
                TEXT_SHORT
            } else {
                TEXT_DIM
            }),
            Node {
                width: Val::Px(70.0),
                ..default()
            },
            Pickable::IGNORE,
        ));
        r.spawn((
            Text::new(techtree::state_label(node.state)),
            font_bold(13.0),
            TextColor(match node.state {
                NodeState::Known => TEXT_DIM,
                NodeState::Ready => LINE_HOT,
                NodeState::Short => TEXT_SHORT,
                NodeState::Blocked => TEXT_DIM,
            }),
            Pickable::IGNORE,
        ));
    });
}

/// Clicks on `Ready` rows: encode the unlock, send it, and say so. The
/// mask and the panel redraw on the `Known` restate that follows — the
/// bit is never set here, `client-core`'s own rule about `SUB_KNOWN`
/// being the authority.
pub fn clicks(
    mut ui: ResMut<Ui>,
    net: NonSend<super::super::Net>,
    buttons: Query<(&Interaction, &NodeButton), Changed<Interaction>>,
) {
    if ui.panel != Panel::Tech {
        return;
    }
    for (interaction, node) in buttons.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        let recipe = node.0;
        let mut buf = [0u8; protocol::MAX_STREAM_MSG_BYTES];
        match protocol::encode_action_unlock(recipe, &mut buf) {
            Ok(len) => match net.session.send_action(&buf[..len]) {
                Ok(()) => ui.say("unlocking…".to_string()),
                Err(e) => ui.say(e.to_string()),
            },
            Err(e) => ui.say(format!("that unlock would not encode ({e:?})")),
        }
    }
}

/// Wheel over the tree column — `craft::scroll`'s clamp on this panel's
/// own container.
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
