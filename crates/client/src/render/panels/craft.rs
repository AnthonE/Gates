//! The craft half of the inventory screen — the frame in
//! `Rust Images/crafting.png`, built out of `crate::ui::craft`'s arithmetic.
//!
//! Four regions, each answering one of the questions the reference frame
//! answers and ours did not (`MENUS.md` §3):
//!
//! - the **rail**, with a live count per bucket, so a filter that would show
//!   nothing says so before it is clicked;
//! - the **browser**: a search box and a grid of recipes, unaffordable rows
//!   dimmed rather than hidden — the player needs to see what to go and get;
//! - the **detail pane**: output, station badge, craft time, the
//!   AMOUNT/ITEM TYPE/TOTAL/HAVE table, a quantity stepper and the button;
//! - the **queue**, with the head job's countdown and cancel on click.
//!
//! Nothing here computes a price or a time. Every number on the screen comes
//! from a function in `crate::ui::craft`, which is tested headlessly — the
//! panel's job is to put them somewhere a person can read.

use bevy::prelude::*;
use client_wasm::core::ClientCore;
use sim_core::craft::RecipeDef;

use bevy::input::mouse::{MouseScrollUnit, MouseWheel};

use super::{
    Panel, Ui, BADGE, BROWSER_COLS, BROWSER_GRID_H, CELL_BG, CELL_FULL, CELL_GAP_PX, CELL_PX, LINE,
    LINE_HOT, PANEL_BG, PANEL_H, SCROLL_PX_PER_LINE, TEXT, TEXT_DIM, TEXT_SHORT,
};
use crate::ui::craft::{
    affordable, eta_seconds, ingredients, item_label, rows, seconds, station_label, Row, RAIL,
};

/// The recipe grid, which scrolls.
#[derive(Component)]
pub struct BrowserScroll;

/// A bucket on the left rail; the index is into [`RAIL`].
#[derive(Component)]
pub struct CatButton(pub usize);

/// A recipe in the browser grid.
#[derive(Component)]
pub struct RecipeCell(pub u16);

/// The star on the detail pane.
#[derive(Component)]
pub struct FavStar(pub u16);

/// A quantity step: −1, +1, or [`STEP_MAX`] for "as many as I can pay for".
#[derive(Component)]
pub struct Step(pub i32);

/// The stepper's "all" button, drawn as `▶|` in the reference.
pub const STEP_MAX: i32 = i32::MAX;

/// The CRAFT button.
#[derive(Component)]
pub struct CraftGo;

/// A queue job; the index is its position, which is what `ACT_CANCEL`
/// carries. The queue is dense and the head is 0, so an index is only valid
/// for as long as the queue it was drawn from — which is why a cancel is
/// sent on the click and never latched.
#[derive(Component)]
pub struct CancelJob(pub usize);

/// The rail and the browser.
pub fn build_browser(parent: &mut ChildSpawnerCommands, ui: &Ui, core: &ClientCore) {
    parent
        .spawn((
            Node {
                flex_direction: FlexDirection::Row,
                border: UiRect::all(Val::Px(1.0)),
                ..default()
            },
            BackgroundColor(PANEL_BG),
            BorderColor::all(LINE),
        ))
        .with_children(|panel| {
            rail(panel, ui, core);
            browser(panel, ui, core);
        });
}

fn rail(parent: &mut ChildSpawnerCommands, ui: &Ui, core: &ClientCore) {
    let mut buf: Vec<Row> = Vec::new();
    parent
        .spawn(Node {
            flex_direction: FlexDirection::Column,
            width: Val::Px(150.0),
            padding: UiRect::all(Val::Px(6.0)),
            row_gap: Val::Px(2.0),
            ..default()
        })
        .with_children(|col| {
            for (i, cat) in RAIL.iter().enumerate() {
                // The count is the bucket with the search box applied, so a
                // rail that says 3 and a grid that shows 3 can never
                // disagree — they are the same call.
                rows(
                    &core.recipes,
                    &core.inv,
                    &core.catalog,
                    &ui.facts,
                    &ui.favs,
                    *cat,
                    &ui.query,
                    &mut buf,
                );
                let on = ui.cat == *cat;
                col.spawn((
                    Button,
                    CatButton(i),
                    Node {
                        padding: UiRect::axes(Val::Px(8.0), Val::Px(5.0)),
                        justify_content: JustifyContent::SpaceBetween,
                        flex_direction: FlexDirection::Row,
                        ..default()
                    },
                    BackgroundColor(if on { CELL_FULL } else { Color::NONE }),
                ))
                .with_children(|b| {
                    b.spawn((
                        Text::new(cat.label().to_string()),
                        TextFont {
                            font_size: 12.0,
                            ..default()
                        },
                        TextColor(if on { TEXT } else { TEXT_DIM }),
                        Pickable::IGNORE,
                    ));
                    // A zero is drawn, not hidden: an empty bucket the
                    // player can see is an answer, an absent one is a
                    // question.
                    b.spawn((
                        Text::new(format!("{}", buf.len())),
                        TextFont {
                            font_size: 12.0,
                            ..default()
                        },
                        TextColor(TEXT_DIM),
                        Pickable::IGNORE,
                    ));
                });
            }
        });
}

fn browser(parent: &mut ChildSpawnerCommands, ui: &Ui, core: &ClientCore) {
    let mut list: Vec<Row> = Vec::new();
    rows(
        &core.recipes,
        &core.inv,
        &core.catalog,
        &ui.facts,
        &ui.favs,
        ui.cat,
        &ui.query,
        &mut list,
    );

    parent
        .spawn(Node {
            flex_direction: FlexDirection::Column,
            width: Val::Px(BROWSER_COLS as f32 * (CELL_PX + CELL_GAP_PX) + 20.0),
            height: Val::Px(PANEL_H),
            padding: UiRect::all(Val::Px(8.0)),
            row_gap: Val::Px(6.0),
            justify_content: JustifyContent::SpaceBetween,
            ..default()
        })
        .with_children(|col| {
            col.spawn((
                BrowserScroll,
                Node {
                    display: Display::Grid,
                    grid_template_columns: RepeatedGridTrack::px(BROWSER_COLS, CELL_PX),
                    row_gap: Val::Px(CELL_GAP_PX),
                    column_gap: Val::Px(CELL_GAP_PX),
                    height: Val::Px(BROWSER_GRID_H),
                    align_content: AlignContent::Start,
                    overflow: Overflow::scroll_y(),
                    ..default()
                },
            ))
            .with_children(|g| {
                if list.is_empty() {
                    // The dark-panel rule: say what would fill it.
                    g.spawn((
                        Text::new(if core.recipes.recipe_count == 0 {
                            "no recipes yet - the shard is still sending them".to_string()
                        } else {
                            "nothing here matches".to_string()
                        }),
                        TextFont {
                            font_size: 12.0,
                            ..default()
                        },
                        TextColor(TEXT_DIM),
                    ));
                }
                for row in list.iter() {
                    recipe_cell(g, ui, core, row);
                }
            });

            // The search box. Not a widget — a rectangle showing what the
            // keyboard has put in `ui.query`, because Bevy has no text input
            // and one built here would be a text-editing engine in a menu.
            col.spawn((
                Node {
                    padding: UiRect::axes(Val::Px(8.0), Val::Px(6.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    ..default()
                },
                BackgroundColor(CELL_BG),
                BorderColor::all(LINE),
            ))
            .with_children(|b| {
                let empty = ui.query.is_empty();
                b.spawn((
                    Text::new(if empty {
                        "Search...".to_string()
                    } else {
                        ui.query.clone()
                    }),
                    TextFont {
                        font_size: 13.0,
                        ..default()
                    },
                    TextColor(if empty { TEXT_DIM } else { TEXT }),
                ));
            });
        });
}

fn recipe_cell(parent: &mut ChildSpawnerCommands, ui: &Ui, core: &ClientCore, row: &Row) {
    let can = row.affordable > 0;
    let picked = ui.selected == Some(row.recipe);
    parent
        .spawn((
            Button,
            RecipeCell(row.recipe),
            Node {
                width: Val::Px(CELL_PX),
                height: Val::Px(CELL_PX),
                border: UiRect::all(Val::Px(1.0)),
                padding: UiRect::all(Val::Px(3.0)),
                overflow: Overflow::clip(),
                ..default()
            },
            BackgroundColor(if can { CELL_FULL } else { CELL_BG }),
            BorderColor::all(if picked { LINE_HOT } else { LINE }),
        ))
        .with_children(|c| {
            c.spawn((
                Text::new(item_label(&core.catalog, row.output)),
                TextFont {
                    font_size: 10.0,
                    ..default()
                },
                // Dimmed rather than hidden — the reference greys what you
                // cannot afford so you can see what to go and get.
                TextColor(if can { TEXT } else { TEXT_DIM }),
                Pickable::IGNORE,
            ));
        });
}

/// The right-hand pane: what one craft of the selected recipe costs.
pub fn build_detail(parent: &mut ChildSpawnerCommands, ui: &Ui, core: &ClientCore) {
    parent
        .spawn((
            Node {
                width: Val::Px(400.0),
                height: Val::Px(PANEL_H),
                padding: UiRect::all(Val::Px(12.0)),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(8.0),
                border: UiRect::all(Val::Px(1.0)),
                ..default()
            },
            BackgroundColor(PANEL_BG),
            BorderColor::all(LINE),
        ))
        .with_children(|pane| {
            let Some(recipe) = ui.selected else {
                pane.spawn((
                    Text::new("pick something to craft"),
                    TextFont {
                        font_size: 14.0,
                        ..default()
                    },
                    TextColor(TEXT_DIM),
                ));
                return;
            };
            let Some(def) = core.recipes.recipes.get(recipe as usize) else {
                return;
            };
            detail_body(pane, ui, core, recipe, def);
        });
}

fn detail_body(
    pane: &mut ChildSpawnerCommands,
    ui: &Ui,
    core: &ClientCore,
    recipe: u16,
    def: &RecipeDef,
) {
    let count = ui.count.max(1);

    // Title row: what it makes, and how long it takes.
    pane.spawn(Node {
        flex_direction: FlexDirection::Row,
        justify_content: JustifyContent::SpaceBetween,
        align_items: AlignItems::Center,
        ..default()
    })
    .with_children(|row| {
        row.spawn((
            Text::new(item_label(&core.catalog, def.output).to_uppercase()),
            TextFont {
                font_size: 20.0,
                ..default()
            },
            TextColor(TEXT),
        ));
        row.spawn((
            Text::new(format!("{:.1}s", seconds(def, count))),
            TextFont {
                font_size: 14.0,
                ..default()
            },
            TextColor(TEXT_DIM),
        ));
    });

    if let Some(badge) = station_label(def.station) {
        pane.spawn((
            Node {
                padding: UiRect::axes(Val::Px(6.0), Val::Px(2.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.30, 0.27, 0.08, 0.9)),
        ))
        .with_children(|b| {
            b.spawn((
                Text::new(badge.to_string()),
                TextFont {
                    font_size: 11.0,
                    ..default()
                },
                TextColor(BADGE),
            ));
        });
    }

    pane.spawn((
        Button,
        FavStar(recipe),
        Node {
            padding: UiRect::axes(Val::Px(6.0), Val::Px(3.0)),
            align_self: AlignSelf::FlexStart,
            border: UiRect::all(Val::Px(1.0)),
            ..default()
        },
        BorderColor::all(LINE),
    ))
    .with_children(|b| {
        let on = ui.favs.contains(&recipe);
        b.spawn((
            Text::new(if on { "* FAVOURITED" } else { "* favourite" }.to_string()),
            TextFont {
                font_size: 11.0,
                ..default()
            },
            TextColor(if on { BADGE } else { TEXT_DIM }),
            Pickable::IGNORE,
        ));
    });

    // The ingredient table, headed exactly as the reference heads it.
    let (lines, n) = ingredients(def, count, &core.inv);
    pane.spawn(Node {
        flex_direction: FlexDirection::Column,
        row_gap: Val::Px(3.0),
        margin: UiRect::top(Val::Px(6.0)),
        ..default()
    })
    .with_children(|table| {
        table_row(
            table,
            "AMOUNT",
            "ITEM TYPE",
            "TOTAL",
            "HAVE",
            TEXT_DIM,
            11.0,
        );
        if n == 0 {
            table_row(table, "-", "no materials", "-", "-", TEXT_DIM, 12.0);
        }
        for line in lines.iter().take(n) {
            table_row(
                table,
                &format!("{}", line.amount),
                &item_label(&core.catalog, line.item),
                &format!("{}", line.total),
                &format!("{}", line.have),
                if line.short() { TEXT_SHORT } else { TEXT },
                12.0,
            );
        }
    });

    // The stepper and the button.
    let max = affordable(def, &core.inv);
    pane.spawn(Node {
        flex_direction: FlexDirection::Row,
        column_gap: Val::Px(6.0),
        align_items: AlignItems::Center,
        margin: UiRect::top(Val::Px(10.0)),
        ..default()
    })
    .with_children(|row| {
        step_button(row, -1, "-");
        row.spawn((
            Node {
                width: Val::Px(70.0),
                padding: UiRect::axes(Val::Px(8.0), Val::Px(5.0)),
                justify_content: JustifyContent::Center,
                border: UiRect::all(Val::Px(1.0)),
                ..default()
            },
            BackgroundColor(CELL_BG),
            BorderColor::all(LINE),
        ))
        .with_children(|b| {
            b.spawn((
                Text::new(format!("{count}")),
                TextFont {
                    font_size: 14.0,
                    ..default()
                },
                TextColor(TEXT),
            ));
        });
        step_button(row, 1, "+");
        step_button(row, STEP_MAX, ">|");

        let can = max >= count as u32;
        row.spawn((
            Button,
            CraftGo,
            Node {
                width: Val::Px(150.0),
                padding: UiRect::axes(Val::Px(10.0), Val::Px(7.0)),
                justify_content: JustifyContent::Center,
                border: UiRect::all(Val::Px(1.0)),
                margin: UiRect::left(Val::Px(10.0)),
                ..default()
            },
            BackgroundColor(if can { CELL_FULL } else { CELL_BG }),
            BorderColor::all(if can { LINE_HOT } else { LINE }),
        ))
        .with_children(|b| {
            b.spawn((
                Text::new("CRAFT".to_string()),
                TextFont {
                    font_size: 15.0,
                    ..default()
                },
                TextColor(if can { TEXT } else { TEXT_DIM }),
                Pickable::IGNORE,
            ));
        });
    });

    // The ceiling, stated. A stepper that silently stops climbing is a
    // stepper the player thinks is broken.
    pane.spawn((
        Text::new(format!("you can pay for {max}")),
        TextFont {
            font_size: 11.0,
            ..default()
        },
        TextColor(TEXT_DIM),
    ));
}

fn step_button(parent: &mut ChildSpawnerCommands, by: i32, glyph: &str) {
    parent
        .spawn((
            Button,
            Step(by),
            Node {
                width: Val::Px(34.0),
                padding: UiRect::axes(Val::Px(8.0), Val::Px(5.0)),
                justify_content: JustifyContent::Center,
                border: UiRect::all(Val::Px(1.0)),
                ..default()
            },
            BackgroundColor(CELL_BG),
            BorderColor::all(LINE),
        ))
        .with_children(|b| {
            b.spawn((
                Text::new(glyph.to_string()),
                TextFont {
                    font_size: 14.0,
                    ..default()
                },
                TextColor(TEXT),
                Pickable::IGNORE,
            ));
        });
}

#[allow(clippy::too_many_arguments)]
fn table_row(
    parent: &mut ChildSpawnerCommands,
    amount: &str,
    item: &str,
    total: &str,
    have: &str,
    colour: Color,
    size: f32,
) {
    parent
        .spawn(Node {
            flex_direction: FlexDirection::Row,
            ..default()
        })
        .with_children(|row| {
            for (text, width) in [(amount, 66.0), (item, 168.0), (total, 66.0), (have, 66.0)] {
                row.spawn((
                    Node {
                        width: Val::Px(width),
                        ..default()
                    },
                    Text::new(text.to_string()),
                    TextFont {
                        font_size: size,
                        ..default()
                    },
                    TextColor(colour),
                ));
            }
        });
}

/// The queue strip. Head first, its countdown live.
pub fn build_queue(parent: &mut ChildSpawnerCommands, _ui: &Ui, core: &ClientCore) {
    parent
        .spawn((
            Node {
                width: Val::Px(970.0),
                min_height: Val::Px(46.0),
                padding: UiRect::all(Val::Px(8.0)),
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: Val::Px(8.0),
                border: UiRect::all(Val::Px(1.0)),
                ..default()
            },
            BackgroundColor(PANEL_BG),
            BorderColor::all(LINE),
        ))
        .with_children(|strip| {
            strip.spawn((
                Text::new("CRAFTING QUEUE".to_string()),
                TextFont {
                    font_size: 13.0,
                    ..default()
                },
                TextColor(TEXT_DIM),
            ));
            let n = core.jobs_count as usize;
            if n == 0 {
                strip.spawn((
                    Text::new("empty".to_string()),
                    TextFont {
                        font_size: 12.0,
                        ..default()
                    },
                    TextColor(TEXT_DIM),
                ));
                return;
            }
            for (i, (recipe, remaining)) in core.jobs.iter().take(n).enumerate() {
                let output = core
                    .recipes
                    .recipes
                    .get(*recipe as usize)
                    .map(|d| d.output)
                    .unwrap_or(0);
                strip
                    .spawn((
                        Button,
                        CancelJob(i),
                        Node {
                            padding: UiRect::axes(Val::Px(8.0), Val::Px(5.0)),
                            flex_direction: FlexDirection::Column,
                            border: UiRect::all(Val::Px(1.0)),
                            ..default()
                        },
                        BackgroundColor(CELL_FULL),
                        BorderColor::all(if i == 0 { LINE_HOT } else { LINE }),
                    ))
                    .with_children(|b| {
                        b.spawn((
                            Text::new(format!(
                                "{} x{}",
                                item_label(&core.catalog, output),
                                remaining
                            )),
                            TextFont {
                                font_size: 12.0,
                                ..default()
                            },
                            TextColor(TEXT),
                            Pickable::IGNORE,
                        ));
                        // Only the head has started, so only the head has a
                        // countdown. Drawing one on a queued job would be
                        // inventing a number the sim has not computed.
                        b.spawn((
                            Text::new(if i == 0 {
                                format!(
                                    "{:.1}s  click to cancel",
                                    eta_seconds(core.craft_eta_ticks)
                                )
                            } else {
                                "click to cancel".to_string()
                            }),
                            TextFont {
                                font_size: 10.0,
                                ..default()
                            },
                            TextColor(TEXT_DIM),
                            Pickable::IGNORE,
                        ));
                    });
            }
        });
}

/// Every click the craft panel owns.
// Eight queries because the panel has eight kinds of clickable thing and
// each one is a distinct component. Merging them behind an enum component
// would move the match from the type system into a runtime `if`.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn clicks(
    mut ui: ResMut<Ui>,
    net: NonSend<super::super::Net>,
    cats: Query<(&Interaction, &CatButton), Changed<Interaction>>,
    recipes: Query<(&Interaction, &RecipeCell), Changed<Interaction>>,
    stars: Query<(&Interaction, &FavStar), Changed<Interaction>>,
    steps: Query<(&Interaction, &Step), Changed<Interaction>>,
    go: Query<&Interaction, (Changed<Interaction>, With<CraftGo>)>,
    cancels: Query<(&Interaction, &CancelJob), Changed<Interaction>>,
) {
    if ui.panel != Panel::Inventory {
        return;
    }
    let core = &net.session.core;

    for (interaction, cat) in cats.iter() {
        if *interaction == Interaction::Pressed {
            if let Some(c) = RAIL.get(cat.0) {
                ui.cat = *c;
                ui.dirty = true;
            }
        }
    }

    for (interaction, cell) in recipes.iter() {
        if *interaction == Interaction::Pressed {
            ui.selected = Some(cell.0);
            // A fresh pick starts at one. Carrying the last recipe's count
            // over is how a player queues 40 of something by accident.
            ui.count = 1;
            ui.dirty = true;
        }
    }

    for (interaction, star) in stars.iter() {
        if *interaction == Interaction::Pressed {
            match ui.favs.iter().position(|r| *r == star.0) {
                Some(i) => {
                    ui.favs.remove(i);
                }
                // Bounded, because everything client-driven is (wall 4).
                None if ui.favs.len() < MAX_FAVS => ui.favs.push(star.0),
                None => ui.say("that is as many favourites as this holds"),
            }
            ui.dirty = true;
        }
    }

    for (interaction, step) in steps.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        let Some(recipe) = ui.selected else { continue };
        let Some(def) = core.recipes.recipes.get(recipe as usize) else {
            continue;
        };
        // The ceiling is what the inventory pays for, clamped into `u16`
        // because that is the wire's width for a craft count.
        let max = affordable(def, &core.inv).min(u16::MAX as u32).max(1) as u16;
        ui.count = match step.0 {
            STEP_MAX => max,
            by => (ui.count as i32 + by).clamp(1, max as i32) as u16,
        };
        ui.dirty = true;
    }

    for interaction in go.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        let Some(recipe) = ui.selected else { continue };
        let count = ui.count.max(1);
        let mut buf = [0u8; protocol::MAX_STREAM_MSG_BYTES];
        match protocol::encode_action_craft(recipe, count, &mut buf) {
            Ok(len) => match net.session.send_action(&buf[..len]) {
                // The queue redraws when the sim answers with `CraftQ`. It
                // is not drawn here, because a queue the client wrote is a
                // queue that can disagree with the one being worked.
                Ok(()) => ui.say(format!("queued {count}")),
                Err(e) => ui.say(e.to_string()),
            },
            Err(e) => ui.say(format!("that craft would not encode ({e:?})")),
        }
    }

    for (interaction, job) in cancels.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        let Ok(index) = u16::try_from(job.0) else {
            continue;
        };
        let mut buf = [0u8; protocol::MAX_STREAM_MSG_BYTES];
        match protocol::encode_action_cancel(index, &mut buf) {
            Ok(len) => match net.session.send_action(&buf[..len]) {
                Ok(()) => ui.status.clear(),
                Err(e) => ui.say(e.to_string()),
            },
            Err(e) => ui.say(format!("that cancel would not encode ({e:?})")),
        }
    }
}

/// Wheel over the recipe grid.
///
/// Clamped to the content rather than left to run: Bevy will happily scroll
/// a list off its own end, and a browser scrolled past its last row is a
/// browser that looks empty. `ComputedNode::content_size` is what the layout
/// actually measured, so the bound is the real one and not an estimate from
/// the row count.
pub fn scroll(
    ui: Res<Ui>,
    mut wheel: MessageReader<MouseWheel>,
    mut grids: Query<(&mut ScrollPosition, &ComputedNode), With<BrowserScroll>>,
) {
    if ui.panel != Panel::Inventory {
        wheel.clear();
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
    for (mut pos, computed) in grids.iter_mut() {
        let over = (computed.content_size.y - computed.size.y).max(0.0);
        pos.0.y = (pos.0.y - dy).clamp(0.0, over);
    }
}

/// How many recipes a player may star. A list fed by clicks is a list that
/// needs a cap (wall 4); the number itself is not a knob worth a row.
pub const MAX_FAVS: usize = 32;
