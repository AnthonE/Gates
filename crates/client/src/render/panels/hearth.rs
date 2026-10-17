//! The hearth's column on the inventory page — Rust's tool cupboard: your
//! body and your pack on the left as always, and where a box's LOOT would
//! be, the upkeep. Cost per day, how long the base is protected, the stock
//! itself as cells, and a FEED button.
//!
//! The words and the arithmetic are `crate::ui::hearth`'s; this is nodes.
//! There is no wire of its own: `E` feeds (and the ack is the reading), a
//! stock cell's click is `take_stock`, FEED is another feed. A drag or a
//! right-click of a material onto the page feeds too (`inv::drag_pointer`).

use bevy::prelude::*;
use client_core::core::ClientCore;

use super::{
    font, font_bold, Hover, Panel, Tip, Ui, CELL_BG, CELL_FULL, CELL_GAP_PX, CELL_HOVER, LINE,
    PANEL_BG, SLOT_PX, TEXT, TEXT_DIM, TEXT_SHORT,
};
use crate::render::icons::Icons;
use crate::ui::hearth::{self as words, Row};

/// The column's width: a box's six slots, so the page is the same shape
/// whether you are looting a box or minding a hearth.
const COL_W_PX: f32 = 6.0 * SLOT_PX + 5.0 * CELL_GAP_PX;
/// A cost cell under COST PER 24 HOURS: smaller than a slot, it is a price.
const COST_PX: f32 = 44.0;
/// The upkeep box's fill, a shade under the panel's.
const WELL: Color = Color::srgba(0.125, 0.114, 0.102, 0.96);
/// Protected, and decaying.
const SAFE: Color = Color::srgb(0.60, 0.88, 0.55);
/// FEED, Rust's green.
const FEED_BG: Color = Color::srgb(0.36, 0.47, 0.20);
const FEED_HOT: Color = Color::srgb(0.43, 0.56, 0.25);
const FEED_TEXT: Color = Color::srgb(0.96, 0.94, 0.89);

/// The whole column, or a region of it a drag can be released over: a
/// release here feeds instead of dropping (`inv::drag_pointer`).
#[derive(Component)]
pub struct HearthDrop;

/// A stock cell: clicking takes a feed's worth of row `.0` back.
#[derive(Component)]
pub struct HearthTake(pub u8);

/// The FEED button.
#[derive(Component)]
pub struct HearthFeed;

/// The stock rows of the hearth at `at`, if the latched ack is this
/// hearth's.
pub fn known_rows(core: &ClientCore, at: (u16, u16, u8)) -> Option<&[(u16, u32, u32)]> {
    (core.stock_addr == at && core.stock_count > 0)
        .then(|| &core.stock[..(core.stock_count as usize).min(core.stock.len())])
}

/// The right-hand column of the inventory page while a hearth is open.
/// Stretched to the row's height, so it stands as tall as your pack.
pub fn column(
    row: &mut ChildSpawnerCommands,
    core: &ClientCore,
    icons: &Icons,
    at: (u16, u16, u8),
) {
    let rows = known_rows(core, at);
    row.spawn((
        HearthDrop,
        Node {
            flex_direction: FlexDirection::Column,
            align_self: AlignSelf::Stretch,
            width: Val::Px(COL_W_PX + 22.0),
            padding: UiRect::all(Val::Px(10.0)),
            row_gap: Val::Px(8.0),
            border: UiRect::all(Val::Px(1.0)),
            ..default()
        },
        BackgroundColor(PANEL_BG),
        BorderColor::all(LINE),
    ))
    .with_children(|col| {
        col.spawn((Text::new("LOOT"), font_bold(15.0), TextColor(TEXT)));
        name_bar(col, "HEARTH");
        upkeep_box(col, core, icons, rows);
        if let Some(rows) = rows {
            col.spawn((Text::new("STOCK"), font_bold(12.0), TextColor(TEXT_DIM)));
            col.spawn(Node {
                display: Display::Grid,
                grid_template_columns: RepeatedGridTrack::px(6, SLOT_PX),
                row_gap: Val::Px(CELL_GAP_PX),
                column_gap: Val::Px(CELL_GAP_PX),
                ..default()
            })
            .with_children(|g| {
                for (i, r) in words::rows(rows).into_iter().enumerate() {
                    stock_cell(g, core, icons, i as u8, r);
                }
            });
        }
        // What is left of the column's height, so FEED sits at its foot
        // like the reference's buttons under the loot grid.
        col.spawn(Node {
            flex_grow: 1.0,
            ..default()
        });
        feed_button(col);
    });
}

/// The strip naming what was opened, `inv::name_bar`'s look.
fn name_bar(parent: &mut ChildSpawnerCommands, name: &str) {
    parent
        .spawn((
            Node {
                width: Val::Percent(100.0),
                padding: UiRect::axes(Val::Px(6.0), Val::Px(3.0)),
                ..default()
            },
            BackgroundColor(CELL_BG),
        ))
        .with_children(|bar| {
            bar.spawn((
                Text::new(name.to_string()),
                font_bold(12.0),
                TextColor(TEXT),
                Pickable::IGNORE,
            ));
        });
}

/// UPKEEP: the explainer, what a day costs, and the verdict.
fn upkeep_box(
    parent: &mut ChildSpawnerCommands,
    core: &ClientCore,
    icons: &Icons,
    rows: Option<&[(u16, u32, u32)]>,
) {
    parent
        .spawn((
            Node {
                flex_direction: FlexDirection::Column,
                width: Val::Percent(100.0),
                border: UiRect::all(Val::Px(1.0)),
                ..default()
            },
            BackgroundColor(WELL),
            BorderColor::all(LINE),
        ))
        .with_children(|b| {
            // The head strip.
            b.spawn((
                Node {
                    padding: UiRect::axes(Val::Px(8.0), Val::Px(5.0)),
                    ..default()
                },
                BackgroundColor(CELL_BG),
            ))
            .with_children(|h| {
                h.spawn((
                    Text::new("UPKEEP MANAGEMENT"),
                    font_bold(13.0),
                    TextColor(TEXT),
                    Pickable::IGNORE,
                ));
            });
            // The (i) and the sentence beside it.
            b.spawn(Node {
                flex_direction: FlexDirection::Row,
                column_gap: Val::Px(10.0),
                align_items: AlignItems::Center,
                padding: UiRect::all(Val::Px(10.0)),
                ..default()
            })
            .with_children(|r| {
                r.spawn((
                    Node {
                        width: Val::Px(26.0),
                        height: Val::Px(26.0),
                        flex_shrink: 0.0,
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        border: UiRect::all(Val::Px(2.0)),
                        border_radius: BorderRadius::MAX,
                        ..default()
                    },
                    BorderColor::all(TEXT_DIM),
                    children![(Text::new("i"), font_bold(15.0), TextColor(TEXT_DIM))],
                ));
                r.spawn((
                    Text::new(words::BLURB),
                    font(12.0),
                    TextColor(TEXT_DIM),
                    Node {
                        flex_shrink: 1.0,
                        ..default()
                    },
                ));
            });
            // The rule between the explainer and the numbers.
            b.spawn((
                Node {
                    height: Val::Px(1.0),
                    ..default()
                },
                BackgroundColor(LINE),
            ));
            b.spawn(Node {
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: Val::Px(8.0),
                padding: UiRect::all(Val::Px(12.0)),
                ..default()
            })
            .with_children(|n| {
                let Some(rows) = rows else {
                    n.spawn((
                        Text::new(words::NO_REPORT),
                        font(13.0),
                        TextColor(TEXT),
                        TextLayout::new_with_justify(Justify::Center),
                    ));
                    return;
                };
                let billed: Vec<Row> = words::rows(rows)
                    .into_iter()
                    .filter(|r| r.per_day > 0)
                    .collect();
                if !billed.is_empty() {
                    n.spawn((
                        Text::new("COST PER 24 HOURS"),
                        font_bold(12.0),
                        TextColor(TEXT_DIM),
                    ));
                    n.spawn(Node {
                        flex_direction: FlexDirection::Row,
                        column_gap: Val::Px(14.0),
                        ..default()
                    })
                    .with_children(|c| {
                        for r in billed {
                            cost_cell(c, core, icons, r);
                        }
                    });
                }
                let warn = words::minutes_left(rows) == Some(0);
                n.spawn((
                    Text::new(words::status_line(rows)),
                    font_bold(17.0),
                    TextColor(if warn { TEXT_SHORT } else { SAFE }),
                    TextLayout::new_with_justify(Justify::Center),
                ));
            });
        });
}

/// One price: the material's picture over what a day of the base takes.
fn cost_cell(parent: &mut ChildSpawnerCommands, core: &ClientCore, icons: &Icons, r: Row) {
    let name = crate::ui::craft::item_label(&core.catalog, r.item);
    parent
        .spawn((
            Node {
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: Val::Px(2.0),
                ..default()
            },
            Tip(format!("{name} · {} a day", r.per_day)),
        ))
        .with_children(|c| {
            picture(c, icons, &name, COST_PX, 1.0);
            c.spawn((
                Text::new(words::count_label(r.per_day)),
                font_bold(13.0),
                TextColor(TEXT),
                Pickable::IGNORE,
            ));
        });
}

/// One stock row as a slot: the material and how much the hearth holds.
/// A click takes a feed's worth back into the pack.
fn stock_cell(parent: &mut ChildSpawnerCommands, core: &ClientCore, icons: &Icons, i: u8, r: Row) {
    let name = crate::ui::craft::item_label(&core.catalog, r.item);
    let filled = r.units > 0;
    let rest = if filled { CELL_FULL } else { CELL_BG };
    parent
        .spawn((
            Button,
            HearthTake(i),
            Node {
                width: Val::Px(SLOT_PX),
                height: Val::Px(SLOT_PX),
                border: UiRect::all(Val::Px(1.0)),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                overflow: Overflow::clip(),
                ..default()
            },
            BackgroundColor(rest),
            Hover {
                rest,
                hot: CELL_HOVER,
            },
            BorderColor::all(LINE),
            Tip(if filled {
                format!(
                    "{name} · {} in the hearth · {} a day · click to take {} back",
                    r.units,
                    r.per_day,
                    sim_core::deploy::FEED_CHUNK.min(r.units)
                )
            } else {
                format!("{name} · none in the hearth · {} a day", r.per_day)
            }),
        ))
        .with_children(|c| {
            // An empty row still shows what goes in it, faded.
            picture(
                c,
                icons,
                &name,
                SLOT_PX - 10.0,
                if filled { 1.0 } else { 0.3 },
            );
            if filled {
                c.spawn((
                    Text::new(words::count_label(r.units)),
                    font_bold(12.0),
                    TextColor(TEXT),
                    crate::render::ui::TEXT_SHADOW,
                    Node {
                        position_type: PositionType::Absolute,
                        right: Val::Px(2.0),
                        bottom: Val::Px(1.0),
                        ..default()
                    },
                    Pickable::IGNORE,
                ));
            }
        });
}

/// An item's picture at `px`, or its name where no icon was baked.
fn picture(parent: &mut ChildSpawnerCommands, icons: &Icons, name: &str, px: f32, alpha: f32) {
    match icons.item(name) {
        Some(image) => parent.spawn((
            Node {
                width: Val::Px(px),
                height: Val::Px(px),
                ..default()
            },
            ImageNode {
                image,
                color: Color::WHITE.with_alpha(alpha),
                ..default()
            },
            Pickable::IGNORE,
        )),
        None => parent.spawn((
            Text::new(crate::ui::craft::cell_abbrev(
                name,
                crate::ui::craft::CELL_LINE_CHARS,
            )),
            font_bold(10.0),
            TextColor(TEXT.with_alpha(alpha)),
            Pickable::IGNORE,
        )),
    };
}

fn feed_button(parent: &mut ChildSpawnerCommands) {
    parent
        .spawn((
            Button,
            HearthFeed,
            Node {
                width: Val::Percent(100.0),
                padding: UiRect::axes(Val::Px(14.0), Val::Px(10.0)),
                justify_content: JustifyContent::Center,
                ..default()
            },
            BackgroundColor(FEED_BG),
            Hover {
                rest: FEED_BG,
                hot: FEED_HOT,
            },
            Tip("puts up to 500 of each upkeep material you carry into the hearth".into()),
        ))
        .with_children(|b| {
            b.spawn((
                Text::new("FEED 500 OF EACH"),
                font_bold(15.0),
                TextColor(FEED_TEXT),
                Pickable::IGNORE,
            ));
        });
}

/// Feed the hearth at `at` and say so on the page's status line.
pub fn feed(ui: &mut Ui, net: &super::super::Net, at: (u16, u16, u8), said: &str) {
    let (cx, cz, level) = at;
    let mut buf = [0u8; protocol::MAX_STREAM_MSG_BYTES];
    match protocol::encode_action_feed(cx, cz, level, &mut buf) {
        Ok(len) => match net.session.send_action(&buf[..len]) {
            Ok(()) => {
                ui.say(said.to_string());
                ui.sound = Some(crate::sound::Cue::ItemMove);
            }
            Err(e) => ui.say(e.to_string()),
        },
        Err(e) => ui.say(format!("the feed would not encode ({e:?})")),
    }
}

/// A stock cell's click takes a feed's worth back; FEED feeds.
pub fn clicks(
    mut ui: ResMut<Ui>,
    net: NonSend<super::super::Net>,
    hearth: Res<super::super::verbs::HearthView>,
    takes: Query<(&Interaction, &HearthTake), Changed<Interaction>>,
    feeds: Query<&Interaction, (Changed<Interaction>, With<HearthFeed>)>,
) {
    let Some(at) = hearth.0 else {
        return;
    };
    if ui.panel != Panel::Inventory {
        return;
    }
    for (interaction, take) in takes.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        let core = &net.session.core;
        let Some(&(item, units, _)) = known_rows(core, at).and_then(|r| r.get(take.0 as usize))
        else {
            continue;
        };
        if units == 0 {
            ui.say("there is none of that in the hearth");
            continue;
        }
        let name = crate::ui::craft::item_label(&core.catalog, item).to_lowercase();
        let (cx, cz, level) = at;
        let mut buf = [0u8; protocol::MAX_STREAM_MSG_BYTES];
        match protocol::encode_action_take_stock(cx, cz, level, take.0, &mut buf) {
            Ok(len) => match net.session.send_action(&buf[..len]) {
                Ok(()) => {
                    let n = sim_core::deploy::FEED_CHUNK.min(units);
                    ui.say(format!("took {n} {name} out of the hearth"));
                    ui.sound = Some(crate::sound::Cue::ItemMove);
                }
                Err(e) => ui.say(e.to_string()),
            },
            Err(e) => ui.say(format!("the take would not encode ({e:?})")),
        }
    }
    if feeds.iter().any(|i| *i == Interaction::Pressed) {
        feed(&mut ui, &net, at, "fed the hearth up to 500 of each");
    }
}
