//! The crafting page (`Q`) — the frame in the reference `crafting.png`,
//! built out of `crate::ui::craft`'s arithmetic — and the inventory page's
//! quick-craft column.
//!
//! Four regions, each answering one of the questions the reference frame
//! answers and ours did not (`MENUS.md` §3):
//!
//! - the **rail**, with a live count per bucket, so a filter that would show
//!   nothing says so before it is clicked;
//! - the **browser**: a search box and a grid of recipe pictures,
//!   unaffordable ones greyed rather than hidden — the player needs to see
//!   what to go and get — a padlock over the ones not yet learned and a star
//!   on the favourites, Rust's three marks;
//! - the **detail pane**: the picture, the name, the station, the craft time
//!   beside a clock and the yield, the AMOUNT/ITEM TYPE/TOTAL/HAVE table with
//!   a picture per ingredient, a quantity stepper and the button;
//! - the **queue**, as Rust draws it: a tile per job, the one being worked
//!   green with its countdown and progress, a cross to cancel.
//!
//! Nothing here computes a price or a time. Every number on the screen comes
//! from a function in `crate::ui::craft`, which is tested headlessly — the
//! panel's job is to put them somewhere a person can read. The only numbers
//! that move between redraws — the head job's seconds and its progress —
//! are written in place by [`queue_tick`], never by a redraw.

use bevy::prelude::*;
use client_core::core::ClientCore;
use sim_core::craft::RecipeDef;

use bevy::input::mouse::{MouseScrollUnit, MouseWheel};

use super::{
    font, font_bold, Hover, Panel, Ui, ACCENT, BADGE, BROWSER_COLS, BROWSER_GRID_H, CELL_BG,
    CELL_FULL, CELL_GAP_PX, CELL_HOVER, CELL_PX, LINE, LINE_HOT, PANEL_BG, PANEL_H,
    SCROLL_PX_PER_LINE, SLOT_PX, TEXT, TEXT_DIM, TEXT_SHORT,
};
use crate::render::icons::{Icons, LOCK_TINT, PICTURE, PICTURE_DIM};
use crate::ui::craft::{
    affordable, cell_abbrev, countdown_label, ingredients, item_label, rows, seconds,
    station_label, Row, CELL_LINE_CHARS, RAIL,
};

/// Rust's favourite star, gold (≈#C8B02A).
const STAR: Color = Color::srgb(0.784, 0.690, 0.165);
/// A rail count on an unselected bucket: Rust draws its counts blue.
const COUNT: Color = Color::srgb(0.47, 0.69, 0.89);
/// The job being worked, and the pointer on it: Rust's interface green
/// (#738D45), the reference's *"the currently crafting item is green"*.
const QUEUE_HEAD: Color = Color::srgba(0.451, 0.553, 0.271, 0.92);
const QUEUE_HEAD_HOT: Color = Color::srgba(0.53, 0.64, 0.33, 0.96);
/// The progress along the head tile's foot.
const QUEUE_FILL: Color = Color::srgba(0.86, 0.95, 0.70, 0.95);
/// The cancel cross on a queue tile — Rust's orange (#C26D33).
const CANCEL: Color = Color::srgb(0.761, 0.427, 0.200);
/// The empty queue's label, faded the way Rust fades it.
const QUEUE_EMPTY: Color = Color::srgba(0.74, 0.72, 0.68, 0.30);
/// Queue tile edge, px. With the strip's padding this is the 56 px row
/// `PANEL_H`'s 720p budget leaves the queue.
const QUEUE_TILE_PX: f32 = 46.0;
/// Most recipes the quick-craft column offers: Rust's 6 × 3, which does
/// not scroll — and a bound on a list fed by the inventory (wall 4).
pub const QUICK_MAX: usize = 18;
/// What a middle-click on a quick-craft cell queues, Rust's.
const QUICK_MIDDLE: u32 = 5;
/// Quick-craft columns: the pack's own six, so the column lines up with it.
const QUICK_COLS: u16 = 6;

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

/// A skin chip on the detail pane (skins v0): the catalog id it picks, 0 for
/// the item's own look. Only owned skins are buttons.
#[derive(Component)]
pub struct SkinChip(pub u16);

/// A queue job; the index is its position, which is what `ACT_CANCEL`
/// carries. The queue is dense and the head is 0, so an index is only valid
/// for as long as the queue it was drawn from — which is why a cancel is
/// sent on the click and never latched.
#[derive(Component)]
pub struct CancelJob(pub usize);

/// The search box: a click gives it the keyboard (`Ui::search_focus`).
#[derive(Component)]
pub struct SearchBox;

/// A recipe in the inventory page's quick-craft column: a click queues one,
/// a middle-click five, a right-click takes its last job out of the queue.
#[derive(Component)]
pub struct QuickCraft(pub u16);

/// The head tile's countdown, written by [`queue_tick`].
#[derive(Component)]
pub struct QueueEta;

/// The head tile's progress bar, sized by [`queue_tick`].
#[derive(Component)]
pub struct QueueFill;

/// The crafting page (`Q`) — Rust's crafting menu, on its own: the tabs,
/// the rail and the browser beside the detail pane, and the queue under
/// both. Your pack is on the other page; the ingredient table's HAVE column
/// is what this one needs of it.
pub fn build_screen(commands: &mut Commands, ui: &Ui, core: &ClientCore, icons: &Icons) {
    super::page_root(commands, ui, |zone| {
        // One column so the queue is exactly as wide as the row over it.
        zone.spawn(Node {
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(8.0),
            ..default()
        })
        .with_children(|col| {
            col.spawn(Node {
                flex_direction: FlexDirection::Row,
                column_gap: Val::Px(8.0),
                ..default()
            })
            .with_children(|row| {
                build_browser(row, ui, core, icons);
                build_detail(row, ui, core, icons);
            });
            build_queue(col, ui, core, icons);
        });
        zone.spawn((
            Text::new(
                "click a recipe   -   click the search box to type   -   \
                 Q or Esc closes   -   Tab for your inventory",
            ),
            font(12.0),
            TextColor(TEXT_DIM),
        ));
    });
}

/// The rail and the browser.
pub fn build_browser(parent: &mut ChildSpawnerCommands, ui: &Ui, core: &ClientCore, icons: &Icons) {
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
            browser(panel, ui, core, icons);
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
                    core.known(),
                    *cat,
                    &ui.query,
                    &mut buf,
                );
                let on = ui.cat == *cat;
                // **Blue, and it is the only hue on the panel.** The
                // selected category was a lighter grey block, which on a
                // panel made entirely of lighter and darker grey blocks
                // says "hovered" rather than "chosen". `crafting.png`
                // selects in `#3982ba` — `ui::ACCENT` — against warm grey
                // everywhere else, so the eye finds it without reading.
                let rest = if on { ACCENT } else { Color::NONE };
                col.spawn((
                    Button,
                    CatButton(i),
                    Node {
                        padding: UiRect::axes(Val::Px(8.0), Val::Px(5.0)),
                        justify_content: JustifyContent::SpaceBetween,
                        flex_direction: FlexDirection::Row,
                        ..default()
                    },
                    BackgroundColor(rest),
                    Hover {
                        rest,
                        hot: if on {
                            crate::render::ui::ACCENT_HOVER
                        } else {
                            CELL_HOVER
                        },
                    },
                ))
                .with_children(|b| {
                    b.spawn((
                        Text::new(cat.label().to_string()),
                        font_bold(12.0),
                        TextColor(if on { TEXT } else { TEXT_DIM }),
                        Pickable::IGNORE,
                    ));
                    // A zero is drawn, not hidden: an empty bucket the
                    // player can see is an answer, an absent one is a
                    // question.
                    b.spawn((
                        Text::new(format!("{}", buf.len())),
                        font_bold(12.0),
                        // On the selected row the count sits on blue, where
                        // a blue count would vanish.
                        TextColor(if on { TEXT } else { COUNT }),
                        Pickable::IGNORE,
                    ));
                });
            }
        });
}

fn browser(parent: &mut ChildSpawnerCommands, ui: &Ui, core: &ClientCore, icons: &Icons) {
    let mut list: Vec<Row> = Vec::new();
    rows(
        &core.recipes,
        &core.inv,
        &core.catalog,
        &ui.facts,
        &ui.favs,
        core.known(),
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
                // Where the player left it: this node is new on every
                // redraw, and a new node starts at the top.
                ScrollPosition(Vec2::new(0.0, ui.browser_scroll)),
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
                        font(12.0),
                        TextColor(TEXT_DIM),
                    ));
                }
                for row in list.iter() {
                    recipe_cell(g, ui, core, icons, row);
                }
            });

            // The search box. Not a widget — a rectangle showing what the
            // keyboard has put in `ui.query`, because Bevy has no text input
            // and one built here would be a text-editing engine in a menu.
            // A click gives it the keyboard and turns it amber, Rust's mark
            // for a live field; Enter, Esc or a click elsewhere hands the
            // keyboard back, so the page's letters are keys again.
            let empty = ui.query.is_empty();
            let live = ui.search_focus;
            col.spawn((
                Button,
                SearchBox,
                Node {
                    padding: UiRect::axes(Val::Px(8.0), Val::Px(6.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    ..default()
                },
                BackgroundColor(CELL_BG),
                Hover::on(CELL_BG),
                BorderColor::all(if live { LINE_HOT } else { LINE }),
            ))
            .with_children(|b| {
                b.spawn((
                    Text::new(match (live, empty) {
                        (true, _) => format!("{}|", ui.query),
                        (false, true) => "Search...".to_string(),
                        (false, false) => ui.query.clone(),
                    }),
                    font(13.0),
                    TextColor(if empty && !live { TEXT_DIM } else { TEXT }),
                    Pickable::IGNORE,
                ));
            });
        });
}

fn recipe_cell(
    parent: &mut ChildSpawnerCommands,
    ui: &Ui,
    core: &ClientCore,
    icons: &Icons,
    row: &Row,
) {
    // Locked outranks unaffordable: a blueprint you have not learned
    // cannot be paid for at any price, so the cell must not read as "go
    // and get more wood" (research v0).
    let can = row.affordable > 0 && !row.locked;
    let picked = ui.selected == Some(row.recipe);
    let rest = if can { CELL_FULL } else { CELL_BG };
    let name = item_label(&core.catalog, row.output);
    // The name and what stands between the player and it — the cell says
    // the same in grey and a padlock, and this says it in words.
    let tip = if row.locked {
        format!("{name} · not learned yet")
    } else if row.affordable == 0 {
        format!("{name} · need materials")
    } else {
        format!("{name} · can make {}", row.affordable)
    };
    parent
        .spawn((
            Button,
            RecipeCell(row.recipe),
            super::Tip(tip),
            Node {
                width: Val::Px(CELL_PX),
                height: Val::Px(CELL_PX),
                border: UiRect::all(Val::Px(1.0)),
                padding: UiRect::all(Val::Px(3.0)),
                overflow: Overflow::clip(),
                ..default()
            },
            BackgroundColor(rest),
            Hover::on(rest),
            BorderColor::all(if picked { LINE_HOT } else { LINE }),
        ))
        .with_children(|c| {
            // The recipe's output, as its picture. Greyed rather than
            // hidden when it cannot be made — the reference greys what you
            // cannot afford so you can see what to go and get.
            match icons.item(&name) {
                Some(image) => c.spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(4.0),
                        top: Val::Px(4.0),
                        width: Val::Px(CELL_PX - 10.0),
                        height: Val::Px(CELL_PX - 10.0),
                        ..default()
                    },
                    ImageNode {
                        image,
                        color: if can { PICTURE } else { PICTURE_DIM },
                        ..default()
                    },
                    Pickable::IGNORE,
                )),
                // No icon baked: the word, abbreviated to the cell
                // (`ui::craft::cell_abbrev`) — the clip alone cuts
                // mid-glyph, and `Workbenc` reads as a defect.
                None => c.spawn((
                    Text::new(cell_abbrev(&name, CELL_LINE_CHARS)),
                    font_bold(10.0),
                    TextColor(if can { TEXT } else { TEXT_DIM }),
                    Pickable::IGNORE,
                )),
            };
            // A locked row wears a padlock, because a dim cell alone is the
            // same picture as an unaffordable one and the two want opposite
            // actions from the player: one says farm, this says go and
            // learn it. The word it replaces had to be read; a padlock is
            // seen.
            if row.locked {
                lock(c, icons, 18.0, (CELL_PX - 2.0 - 18.0) * 0.5);
            }
            if ui.favs.contains(&row.recipe) {
                if let Some(star) = icons.glyph("ui_star") {
                    c.spawn((
                        Node {
                            position_type: PositionType::Absolute,
                            right: Val::Px(2.0),
                            top: Val::Px(2.0),
                            width: Val::Px(11.0),
                            height: Val::Px(11.0),
                            ..default()
                        },
                        ImageNode::new(star).with_color(STAR),
                        Pickable::IGNORE,
                    ));
                }
            }
        });
}

/// The padlock over a picture not yet learned, `px` square at `at` from the
/// cell's inner top-left — or the word, if the glyph is not loaded.
fn lock(c: &mut ChildSpawnerCommands, icons: &Icons, px: f32, at: f32) {
    match icons.glyph("ui_lock") {
        Some(h) => {
            c.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(at),
                    top: Val::Px(at),
                    width: Val::Px(px),
                    height: Val::Px(px),
                    ..default()
                },
                ImageNode::new(h).with_color(LOCK_TINT),
                Pickable::IGNORE,
            ));
        }
        None => {
            c.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(3.0),
                    bottom: Val::Px(2.0),
                    ..default()
                },
                Text::new("LOCKED"),
                font_bold(8.0),
                TextColor(BADGE),
                Pickable::IGNORE,
            ));
        }
    }
}

/// The right-hand pane: what one craft of the selected recipe costs.
pub fn build_detail(parent: &mut ChildSpawnerCommands, ui: &Ui, core: &ClientCore, icons: &Icons) {
    parent
        .spawn((
            Node {
                width: Val::Px(400.0),
                height: Val::Px(PANEL_H),
                padding: UiRect::all(Val::Px(12.0)),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(7.0),
                border: UiRect::all(Val::Px(1.0)),
                overflow: Overflow::clip(),
                ..default()
            },
            BackgroundColor(PANEL_BG),
            BorderColor::all(LINE),
        ))
        .with_children(|pane| {
            let Some(recipe) = ui.selected else {
                pane.spawn((
                    Text::new("pick something to craft"),
                    font(14.0),
                    TextColor(TEXT_DIM),
                ));
                return;
            };
            let Some(def) = core.recipes.recipes.get(recipe as usize) else {
                return;
            };
            detail_body(pane, ui, core, icons, recipe, def);
        });
}

fn detail_body(
    pane: &mut ChildSpawnerCommands,
    ui: &Ui,
    core: &ClientCore,
    icons: &Icons,
    recipe: u16,
    def: &RecipeDef,
) {
    let count = ui.count.max(1);
    let name = item_label(&core.catalog, def.output);
    let locked = def.blueprint && !sim_core::research::knows(core.known(), recipe);
    let pos = core.predict.position();
    let here = crate::ui::craft::station_here(
        def,
        core.deploys.entries(),
        &core.deploy_defs,
        pos[0],
        pos[2],
    );

    // The head: the picture, what it is and where it is made, and — Rust's
    // top right — how long it takes and how many one craft makes.
    pane.spawn(Node {
        flex_direction: FlexDirection::Row,
        column_gap: Val::Px(10.0),
        align_items: AlignItems::FlexStart,
        ..default()
    })
    .with_children(|head| {
        head.spawn((
            Node {
                width: Val::Px(62.0),
                height: Val::Px(62.0),
                flex_shrink: 0.0,
                border: UiRect::all(Val::Px(1.0)),
                ..default()
            },
            BackgroundColor(CELL_FULL),
            BorderColor::all(LINE),
        ))
        .with_children(|tile| {
            if let Some(image) = icons.item(&name) {
                tile.spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(4.0),
                        top: Val::Px(4.0),
                        width: Val::Px(52.0),
                        height: Val::Px(52.0),
                        ..default()
                    },
                    ImageNode {
                        image,
                        color: if locked { PICTURE_DIM } else { PICTURE },
                        ..default()
                    },
                ));
            }
            if locked {
                lock(tile, icons, 24.0, (62.0 - 2.0 - 24.0) * 0.5);
            }
        });

        head.spawn(Node {
            flex_direction: FlexDirection::Column,
            flex_grow: 1.0,
            row_gap: Val::Px(4.0),
            ..default()
        })
        .with_children(|mid| {
            mid.spawn((
                Text::new(name.to_uppercase()),
                font_bold(19.0),
                TextColor(TEXT),
            ));
            if let Some(badge) = station_label(def.station) {
                // Green where the station stands in reach, red where it
                // does not — the badge is the reason CRAFT is dark.
                mid.spawn((
                    Node {
                        padding: UiRect::axes(Val::Px(6.0), Val::Px(2.0)),
                        align_self: AlignSelf::FlexStart,
                        ..default()
                    },
                    BackgroundColor(if here {
                        Color::srgba(0.30, 0.27, 0.08, 0.9)
                    } else {
                        Color::srgba(0.36, 0.10, 0.08, 0.9)
                    }),
                ))
                .with_children(|b| {
                    b.spawn((
                        Text::new(badge.to_string()),
                        font_bold(11.0),
                        TextColor(if here { BADGE } else { TEXT_SHORT }),
                    ));
                });
            }
            fav_toggle(mid, ui, icons, recipe);
        });

        head.spawn(Node {
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::FlexEnd,
            row_gap: Val::Px(4.0),
            flex_shrink: 0.0,
            ..default()
        })
        .with_children(|right| {
            // The time at the bench the player is standing at (craft rebate
            // v0): a higher rung in reach halves a unit, two quarter it, and
            // the green says the bench is doing that.
            let best = sim_core::deploy::best_bench_in(
                core.deploys.entries(),
                &core.deploy_defs,
                pos[0],
                pos[2],
                sim_core::craft::STATION_RADIUS_M,
            );
            let here = crate::ui::craft::seconds_at(def, count, best);
            let rebated = here < seconds(def, count);
            let tint = if rebated { BADGE } else { TEXT };
            right
                .spawn(Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(4.0),
                    ..default()
                })
                .with_children(|r| {
                    if let Some(clock) = icons.glyph("ui_clock") {
                        r.spawn((
                            Node {
                                width: Val::Px(15.0),
                                height: Val::Px(15.0),
                                ..default()
                            },
                            ImageNode::new(clock).with_color(tint),
                        ));
                    }
                    r.spawn((
                        Text::new(format!("{here:.1}s")),
                        font_bold(15.0),
                        TextColor(tint),
                    ));
                });
            if rebated {
                right.spawn((Text::new("BENCH BONUS"), font_bold(10.0), TextColor(BADGE)));
            }
            // What one craft yields — Rust's circled `1` beside the clock.
            right
                .spawn((
                    Node {
                        padding: UiRect::axes(Val::Px(6.0), Val::Px(1.0)),
                        border: UiRect::all(Val::Px(1.0)),
                        ..default()
                    },
                    BorderColor::all(LINE),
                ))
                .with_children(|y| {
                    y.spawn((
                        Text::new(format!("x{}", def.out_count)),
                        font_bold(12.0),
                        TextColor(TEXT_DIM),
                    ));
                });
        });
    });

    // A locked recipe says where it is learned — the bench tree and the
    // node's price — rather than only that it is locked.
    if let Some(hint) = crate::ui::craft::unlock_hint(&core.research, core.known(), recipe, def) {
        pane.spawn((Text::new(hint), font_bold(11.0), TextColor(TEXT_SHORT)));
    }

    // The ingredient table, headed exactly as the reference heads it, with
    // each ingredient's picture beside its name.
    let (lines, n) = ingredients(def, count, &core.inv);
    pane.spawn(Node {
        flex_direction: FlexDirection::Column,
        row_gap: Val::Px(3.0),
        margin: UiRect::top(Val::Px(2.0)),
        ..default()
    })
    .with_children(|table| {
        table_row(
            table,
            None,
            ["AMOUNT", "ITEM TYPE", "TOTAL", "HAVE"],
            TEXT_DIM,
            11.0,
        );
        if n == 0 {
            table_row(table, None, ["-", "no materials", "-", "-"], TEXT_DIM, 12.0);
        }
        for line in lines.iter().take(n) {
            let label = item_label(&core.catalog, line.item);
            table_row(
                table,
                Some(icons.item(&label)),
                [
                    &format!("{}", line.amount),
                    &label,
                    &format!("{}", line.total),
                    &format!("{}", line.have),
                ],
                if line.short() { TEXT_SHORT } else { TEXT },
                12.0,
            );
        }
    });

    skin_picker(pane, ui, core, def.output);

    // The stepper and the button.
    let max = affordable(def, &core.inv);
    pane.spawn(Node {
        flex_direction: FlexDirection::Row,
        column_gap: Val::Px(6.0),
        align_items: AlignItems::Center,
        margin: UiRect::top(Val::Px(6.0)),
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
                font_bold(14.0),
                TextColor(TEXT),
            ));
        });
        step_button(row, 1, "+");
        step_button(row, STEP_MAX, ">|");

        let can = max >= count as u32 && !locked && here;
        let rest = if can { CELL_FULL } else { CELL_BG };
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
            BackgroundColor(rest),
            // A dead button does not light: the pointer on it is not an
            // invitation.
            Hover {
                rest,
                hot: if can { CELL_HOVER } else { rest },
            },
            BorderColor::all(if can { LINE_HOT } else { LINE }),
        ))
        .with_children(|b| {
            b.spawn((
                Text::new("CRAFT".to_string()),
                font_bold(15.0),
                TextColor(if can { TEXT } else { TEXT_DIM }),
                Pickable::IGNORE,
            ));
        });
    });

    // The ceiling, stated. A stepper that silently stops climbing is a
    // stepper the player thinks is broken.
    pane.spawn((
        Text::new(format!("you can pay for {max}")),
        font(11.0),
        TextColor(TEXT_DIM),
    ));
    if !here {
        pane.spawn((
            Text::new(format!(
                "stand within {} m of it to craft this",
                sim_core::craft::STATION_RADIUS_M
            )),
            font(11.0),
            TextColor(TEXT_SHORT),
        ));
    }
}

/// The favourite toggle under the name: Rust's star and the word.
fn fav_toggle(parent: &mut ChildSpawnerCommands, ui: &Ui, icons: &Icons, recipe: u16) {
    let on = ui.favs.contains(&recipe);
    parent
        .spawn((
            Button,
            FavStar(recipe),
            Node {
                padding: UiRect::axes(Val::Px(6.0), Val::Px(2.0)),
                align_self: AlignSelf::FlexStart,
                align_items: AlignItems::Center,
                column_gap: Val::Px(4.0),
                border: UiRect::all(Val::Px(1.0)),
                ..default()
            },
            BackgroundColor(Color::NONE),
            Hover::on(Color::NONE),
            BorderColor::all(LINE),
        ))
        .with_children(|b| {
            if let Some(star) = icons.glyph("ui_star") {
                b.spawn((
                    Node {
                        width: Val::Px(11.0),
                        height: Val::Px(11.0),
                        ..default()
                    },
                    ImageNode::new(star).with_color(if on { STAR } else { TEXT_DIM }),
                    Pickable::IGNORE,
                ));
            }
            b.spawn((
                Text::new(if on { "FAVOURITED" } else { "FAVOURITE" }.to_string()),
                font_bold(11.0),
                TextColor(if on { STAR } else { TEXT_DIM }),
                Pickable::IGNORE,
            ));
        });
}

/// Rust's craft-menu skin picker (skins v0): the item's own look and every
/// skin you own for it, plus the ones you do not, dimmed with their price —
/// the store is the launcher's, so a skin you have not bought is shown, not
/// offered. Drawn only when some skin fits the output.
fn skin_picker(pane: &mut ChildSpawnerCommands, ui: &Ui, core: &ClientCore, output: u16) {
    let rows: Vec<(usize, protocol::SkinRow, bool)> = core.skins_for(output).collect();
    if rows.is_empty() {
        return;
    }
    pane.spawn((
        Text::new("SKIN".to_string()),
        font_bold(11.0),
        TextColor(TEXT_DIM),
        Node {
            margin: UiRect::top(Val::Px(6.0)),
            ..default()
        },
    ));
    pane.spawn(Node {
        flex_direction: FlexDirection::Row,
        flex_wrap: FlexWrap::Wrap,
        column_gap: Val::Px(6.0),
        row_gap: Val::Px(4.0),
        ..default()
    })
    .with_children(|row| {
        skin_chip(row, 0, "Default".to_string(), None, true, ui.skin == 0);
        for (i, r, owned) in rows {
            let name = String::from_utf8_lossy(core.skins.name(i)).into_owned();
            let label = if owned {
                name
            } else {
                format!("{name} · {}", crate::ui::skins::price_label(&r))
            };
            let tint = crate::ui::skins::tint_factors(r.tint);
            skin_chip(
                row,
                r.catalog,
                label,
                Some(tint),
                owned,
                ui.skin == r.catalog,
            );
        }
    });
}

fn skin_chip(
    parent: &mut ChildSpawnerCommands,
    catalog: u16,
    label: String,
    tint: Option<[f32; 3]>,
    owned: bool,
    picked: bool,
) {
    let rest = if picked { CELL_FULL } else { CELL_BG };
    let mut chip = parent.spawn((
        Node {
            flex_direction: FlexDirection::Row,
            column_gap: Val::Px(5.0),
            align_items: AlignItems::Center,
            padding: UiRect::axes(Val::Px(6.0), Val::Px(3.0)),
            border: UiRect::all(Val::Px(1.0)),
            ..default()
        },
        BackgroundColor(rest),
        BorderColor::all(if picked { LINE_HOT } else { LINE }),
    ));
    if owned {
        chip.insert((Button, SkinChip(catalog), Hover::on(rest)));
    }
    chip.with_children(|c| {
        if let Some([r, g, b]) = tint {
            c.spawn((
                Node {
                    width: Val::Px(10.0),
                    height: Val::Px(10.0),
                    ..default()
                },
                BackgroundColor(Color::srgb(r, g, b)),
                Pickable::IGNORE,
            ));
        }
        c.spawn((
            Text::new(label),
            font_bold(11.0),
            TextColor(if owned { TEXT } else { TEXT_DIM }),
            Pickable::IGNORE,
        ));
    });
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
            Hover::on(CELL_BG),
            BorderColor::all(LINE),
        ))
        .with_children(|b| {
            b.spawn((
                Text::new(glyph.to_string()),
                font_bold(14.0),
                TextColor(TEXT),
                Pickable::IGNORE,
            ));
        });
}

/// One line of the ingredient table: AMOUNT, ITEM TYPE (with its picture
/// when there is one), TOTAL, HAVE. `pic` is `None` for the header row and
/// `Some(None)` for an item whose picture did not load, which keeps the
/// name in line with its neighbours'.
fn table_row(
    parent: &mut ChildSpawnerCommands,
    pic: Option<Option<Handle<Image>>>,
    cells: [&str; 4],
    colour: Color,
    size: f32,
) {
    const ICON_PX: f32 = 16.0;
    parent
        .spawn(Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            ..default()
        })
        .with_children(|row| {
            for (i, (text, width)) in cells.iter().zip([62.0, 172.0, 66.0, 66.0]).enumerate() {
                row.spawn(Node {
                    width: Val::Px(width),
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(5.0),
                    ..default()
                })
                .with_children(|cell| {
                    if i == 1 {
                        if let Some(pic) = &pic {
                            let mut slot = cell.spawn(Node {
                                width: Val::Px(ICON_PX),
                                height: Val::Px(ICON_PX),
                                flex_shrink: 0.0,
                                ..default()
                            });
                            if let Some(h) = pic {
                                slot.insert(ImageNode::new(h.clone()));
                            }
                        }
                    }
                    cell.spawn((
                        Text::new(text.to_string()),
                        font_bold(size),
                        TextColor(colour),
                    ));
                });
            }
        });
}

/// The inventory page's quick-craft column — Rust's: what the pack in
/// front of you can make right now, where you stand, favourites first, 6 × 3
/// and no scroll. A click queues one, a middle-click five, a right-click
/// cancels; how many are queued is on the cell, and the queue itself is on
/// the crafting page and the HUD's craft bar, which is up over this page as
/// Rust's is. Devblog 187's lesson is kept by construction: a cell draws the
/// item's default picture, never a skin.
pub fn build_quick(parent: &mut ChildSpawnerCommands, ui: &Ui, core: &ClientCore, icons: &Icons) {
    let mut all: Vec<Row> = Vec::new();
    rows(
        &core.recipes,
        &core.inv,
        &core.catalog,
        &ui.facts,
        &ui.favs,
        core.known(),
        crate::ui::craft::Cat::All,
        "",
        &mut all,
    );
    // What the pack pays for, learned, and makeable where the player
    // stands — a recipe whose bench is elsewhere is not quick. A cell with
    // jobs in the queue stays, dimmed, once the pack can pay for no more:
    // its count and its right-click cancel went with it, and the grid
    // shifted the next recipe under a cursor that was about to click again.
    all.retain(|r| {
        (r.affordable > 0 || queued(core, r.recipe) > 0)
            && !r.locked
            && makeable_here(core, r.recipe)
    });
    // Favourites first, the bake's order otherwise (a stable sort keeps it).
    all.sort_by_key(|r| !ui.favs.contains(&r.recipe));
    all.truncate(QUICK_MAX);
    let width = QUICK_COLS as f32 * (SLOT_PX + CELL_GAP_PX) - CELL_GAP_PX;

    parent
        .spawn((
            Node {
                flex_direction: FlexDirection::Column,
                padding: UiRect::all(Val::Px(10.0)),
                row_gap: Val::Px(6.0),
                border: UiRect::all(Val::Px(1.0)),
                ..default()
            },
            BackgroundColor(PANEL_BG),
            BorderColor::all(LINE),
        ))
        .with_children(|col| {
            col.spawn((
                Text::new("QUICK CRAFT".to_string()),
                font_bold(15.0),
                TextColor(TEXT),
            ));
            if all.is_empty() {
                col.spawn((
                    Text::new("nothing you carry makes anything here yet".to_string()),
                    font(12.0),
                    TextColor(TEXT_DIM),
                    Node {
                        width: Val::Px(width),
                        ..default()
                    },
                ));
                return;
            }
            col.spawn(Node {
                display: Display::Grid,
                grid_template_columns: RepeatedGridTrack::px(QUICK_COLS, SLOT_PX),
                row_gap: Val::Px(CELL_GAP_PX),
                column_gap: Val::Px(CELL_GAP_PX),
                ..default()
            })
            .with_children(|g| {
                for row in &all {
                    quick_cell(g, ui, core, icons, row);
                }
            });
            col.spawn((
                Text::new(
                    "click crafts one   -   middle-click five   -   right-click cancels"
                        .to_string(),
                ),
                font(11.0),
                TextColor(TEXT_DIM),
                Node {
                    width: Val::Px(width),
                    ..default()
                },
            ));
        });
}

/// Does the station `recipe` needs stand where the player does — nothing, a
/// furnace, or a bench of its rung (`ui::craft::station_here`)?
pub fn makeable_here(core: &ClientCore, recipe: u16) -> bool {
    let pos = core.predict.position();
    core.recipes
        .recipes
        .get(recipe as usize)
        .is_some_and(|def| {
            crate::ui::craft::station_here(
                def,
                core.deploys.entries(),
                &core.deploy_defs,
                pos[0],
                pos[2],
            )
        })
}

/// How many of `recipe` are queued, over every job making it.
fn queued(core: &ClientCore, recipe: u16) -> u32 {
    let n = (core.jobs_count as usize).min(core.jobs.len());
    core.jobs[..n]
        .iter()
        .filter(|(r, _)| *r as u16 == recipe)
        .map(|(_, left)| *left as u32)
        .sum()
}

fn quick_cell(
    parent: &mut ChildSpawnerCommands,
    ui: &Ui,
    core: &ClientCore,
    icons: &Icons,
    row: &Row,
) {
    let name = item_label(&core.catalog, row.output);
    let queued = queued(core, row.recipe);
    // Only queued now: the pack pays for no more, so the cell dims the way
    // the grid dims what it cannot afford, and keeps its count and cancel.
    let can = row.affordable > 0;
    let rest = if can { CELL_FULL } else { CELL_BG };
    let tip = if can {
        format!("{name} · you can make {}", row.affordable)
    } else {
        format!("{name} · {queued} queued · need materials for more")
    };
    parent
        .spawn((
            Button,
            QuickCraft(row.recipe),
            super::Tip(tip),
            Node {
                width: Val::Px(SLOT_PX),
                height: Val::Px(SLOT_PX),
                border: UiRect::all(Val::Px(1.0)),
                ..default()
            },
            BackgroundColor(rest),
            Hover::on(rest),
            BorderColor::all(if queued > 0 { QUEUE_HEAD } else { LINE }),
        ))
        .with_children(|c| {
            match icons.item(&name) {
                Some(image) => c.spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(5.0),
                        top: Val::Px(5.0),
                        width: Val::Px(SLOT_PX - 12.0),
                        height: Val::Px(SLOT_PX - 12.0),
                        ..default()
                    },
                    ImageNode {
                        image,
                        color: if can { PICTURE } else { PICTURE_DIM },
                        ..default()
                    },
                    Pickable::IGNORE,
                )),
                None => c.spawn((
                    Text::new(cell_abbrev(&name, CELL_LINE_CHARS)),
                    font_bold(10.0),
                    TextColor(if can { TEXT } else { TEXT_DIM }),
                    Pickable::IGNORE,
                )),
            };
            if ui.favs.contains(&row.recipe) {
                if let Some(star) = icons.glyph("ui_star") {
                    c.spawn((
                        Node {
                            position_type: PositionType::Absolute,
                            right: Val::Px(2.0),
                            top: Val::Px(2.0),
                            width: Val::Px(11.0),
                            height: Val::Px(11.0),
                            ..default()
                        },
                        ImageNode::new(star).with_color(STAR),
                        Pickable::IGNORE,
                    ));
                }
            }
            // How many are coming, in the queue's green — the answer to a
            // click, on the thing clicked.
            if queued > 0 {
                c.spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        right: Val::Px(2.0),
                        bottom: Val::Px(2.0),
                        padding: UiRect::axes(Val::Px(4.0), Val::Px(0.0)),
                        ..default()
                    },
                    BackgroundColor(QUEUE_HEAD),
                    Pickable::IGNORE,
                ))
                .with_children(|b| {
                    b.spawn((
                        Text::new(queued.to_string()),
                        font_bold(11.0),
                        TextColor(TEXT),
                        Pickable::IGNORE,
                    ));
                });
            }
        });
}

/// The queue strip, Rust's way: a tile per job — its picture, its count, a
/// cross to cancel — the head green, with its countdown and a progress bar
/// that [`queue_tick`] moves in place.
pub fn build_queue(parent: &mut ChildSpawnerCommands, _ui: &Ui, core: &ClientCore, icons: &Icons) {
    let n = core.jobs_count as usize;
    parent
        .spawn((
            Node {
                height: Val::Px(QUEUE_TILE_PX + 10.0),
                padding: UiRect::axes(Val::Px(8.0), Val::Px(4.0)),
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: Val::Px(6.0),
                border: UiRect::all(Val::Px(1.0)),
                ..default()
            },
            BackgroundColor(PANEL_BG),
            BorderColor::all(LINE),
        ))
        .with_children(|strip| {
            if n == 0 {
                // Rust's empty queue: the strip's own name, large and faded.
                strip.spawn((
                    Text::new("CRAFTING QUEUE".to_string()),
                    font_bold(20.0),
                    TextColor(QUEUE_EMPTY),
                ));
                return;
            }
            strip.spawn((
                Text::new("QUEUE".to_string()),
                font_bold(13.0),
                TextColor(TEXT_DIM),
                Node {
                    margin: UiRect::right(Val::Px(4.0)),
                    ..default()
                },
            ));
            for (i, (recipe, remaining)) in core.jobs.iter().take(n).enumerate() {
                let output = core
                    .recipes
                    .recipes
                    .get(*recipe as usize)
                    .map(|d| d.output)
                    .unwrap_or(0);
                queue_tile(strip, core, icons, i, output, *remaining);
            }
            strip.spawn((
                Text::new("click a job to cancel it".to_string()),
                font(10.0),
                TextColor(TEXT_DIM),
                Node {
                    margin: UiRect::left(Val::Px(6.0)),
                    ..default()
                },
            ));
        });
}

fn queue_tile(
    strip: &mut ChildSpawnerCommands,
    core: &ClientCore,
    icons: &Icons,
    i: usize,
    output: u16,
    remaining: u8,
) {
    // Only the head has started, so only the head has a countdown. Drawing
    // one on a queued job would be inventing a number the sim has not
    // computed.
    let head = i == 0;
    let rest = if head { QUEUE_HEAD } else { CELL_FULL };
    let name = item_label(&core.catalog, output);
    strip
        .spawn((
            Button,
            CancelJob(i),
            super::Tip(format!("{name} x{remaining} · click to cancel")),
            Node {
                width: Val::Px(QUEUE_TILE_PX),
                height: Val::Px(QUEUE_TILE_PX),
                border: UiRect::all(Val::Px(1.0)),
                flex_shrink: 0.0,
                ..default()
            },
            BackgroundColor(rest),
            Hover {
                rest,
                hot: if head { QUEUE_HEAD_HOT } else { CELL_HOVER },
            },
            BorderColor::all(if head { LINE_HOT } else { LINE }),
        ))
        .with_children(|t| {
            match icons.item(&name) {
                Some(image) => {
                    t.spawn((
                        Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px(4.0),
                            top: Val::Px(3.0),
                            width: Val::Px(QUEUE_TILE_PX - 10.0),
                            height: Val::Px(QUEUE_TILE_PX - 10.0),
                            ..default()
                        },
                        ImageNode::new(image),
                        Pickable::IGNORE,
                    ));
                }
                None => {
                    t.spawn((
                        Text::new(cell_abbrev(&name, CELL_LINE_CHARS)),
                        font_bold(10.0),
                        TextColor(TEXT),
                        Pickable::IGNORE,
                    ));
                }
            }
            // The cross: the tile is the cancel button, and this is what
            // says so.
            t.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    right: Val::Px(2.0),
                    top: Val::Px(-2.0),
                    ..default()
                },
                Text::new("×"),
                font_bold(14.0),
                TextColor(CANCEL),
                Pickable::IGNORE,
            ));
            // The count top left, clear of the countdown along the foot.
            if remaining > 1 {
                t.spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(2.0),
                        top: Val::Px(0.0),
                        ..default()
                    },
                    Text::new(format!("x{remaining}")),
                    font_bold(10.0),
                    TextColor(TEXT),
                    crate::render::ui::TEXT_SHADOW,
                    Pickable::IGNORE,
                ));
            }
            if head {
                t.spawn((
                    QueueEta,
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(2.0),
                        bottom: Val::Px(3.0),
                        ..default()
                    },
                    Text::new(""),
                    font_bold(10.0),
                    TextColor(TEXT),
                    crate::render::ui::TEXT_SHADOW,
                    Pickable::IGNORE,
                ));
                t.spawn((
                    QueueFill,
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(0.0),
                        bottom: Val::Px(0.0),
                        width: Val::Percent(0.0),
                        height: Val::Px(3.0),
                        ..default()
                    },
                    BackgroundColor(QUEUE_FILL),
                    Pickable::IGNORE,
                ));
            }
        });
}

/// The head job's countdown and progress, every frame, in place — from the
/// same clock the HUD's craft bar reads (`hud::CraftTimer`). The text is
/// rewritten only when the whole second moves, or when a redraw has just
/// spawned a new tile to write it into.
pub fn queue_tick(
    ui: Res<Ui>,
    time: Res<Time<Real>>,
    timer: Res<super::super::hud::CraftTimer>,
    mut last: Local<Option<u32>>,
    mut etas: Query<(&mut Text, Ref<QueueEta>)>,
    mut fills: Query<&mut Node, With<QueueFill>>,
) {
    if !matches!(ui.panel, Panel::Craft | Panel::Inventory) {
        *last = None;
        return;
    }
    let now = time.elapsed_secs_f64();
    let left = timer.0.left(now);
    let secs = left.max(0.0).ceil() as u32;
    let moved = *last != Some(secs);
    *last = Some(secs);
    for (mut text, marker) in etas.iter_mut() {
        if moved || marker.is_added() {
            text.0 = countdown_label(left);
        }
    }
    let w = Val::Percent(timer.0.progress(now) * 100.0);
    for mut node in fills.iter_mut() {
        if node.width != w {
            node.width = w;
        }
    }
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
    chips: Query<(&Interaction, &SkinChip), Changed<Interaction>>,
    search: Query<&Interaction, (Changed<Interaction>, With<SearchBox>)>,
    quick: Query<(&Interaction, &QuickCraft)>,
    mouse: Res<ButtonInput<MouseButton>>,
) {
    if !matches!(ui.panel, Panel::Craft | Panel::Inventory) {
        return;
    }
    let core = &net.session.core;

    // The search box takes the keyboard on a click and gives it back on a
    // click anywhere else (`Ui::search_focus`).
    let into_search = search.iter().any(|i| *i == Interaction::Pressed);
    if into_search && !ui.search_focus {
        ui.search_focus = true;
        ui.dirty = true;
    } else if !into_search && ui.search_focus && mouse.just_pressed(MouseButton::Left) {
        ui.search_focus = false;
        ui.dirty = true;
    }

    // Quick craft, Rust's three buttons: left one, middle five (or what the
    // pack pays for, if less), right cancels the recipe's last job. `Pressed`
    // is Bevy's left button only, so the other two are the button's own
    // press over the hovered cell. The queue answers with `CraftQ`.
    let press = if mouse.just_pressed(MouseButton::Left) {
        Some(MouseButton::Left)
    } else if mouse.just_pressed(MouseButton::Middle) {
        Some(MouseButton::Middle)
    } else if mouse.just_pressed(MouseButton::Right) {
        Some(MouseButton::Right)
    } else {
        None
    };
    let hit = quick
        .iter()
        .find(|(i, _)| !matches!(i, Interaction::None))
        .map(|(_, q)| q.0);
    if let (Some(button), Some(recipe)) = (press, hit) {
        let def = core.recipes.recipes.get(recipe as usize);
        let name = def
            .map(|d| item_label(&core.catalog, d.output))
            .unwrap_or_default();
        let mut buf = [0u8; protocol::MAX_STREAM_MSG_BYTES];
        let encoded = if button == MouseButton::Right {
            let n = (core.jobs_count as usize).min(core.jobs.len());
            match core.jobs[..n]
                .iter()
                .rposition(|(r, _)| *r as u16 == recipe)
            {
                Some(i) => Some((
                    protocol::encode_action_cancel(i as u16, &mut buf),
                    format!("cancelled {} × {name}", core.jobs[i].1),
                )),
                None => {
                    ui.say(format!("no {name} in the queue"));
                    None
                }
            }
        } else {
            let can = def.map(|d| affordable(d, &core.inv)).unwrap_or(0);
            if can == 0 {
                // A cell kept for its queue: the pack pays for no more.
                ui.say(format!("need materials for another {name}"));
                None
            } else {
                let want = if button == MouseButton::Middle {
                    QUICK_MIDDLE.min(can).max(1)
                } else {
                    1
                };
                Some((
                    protocol::encode_action_craft(recipe, want as u16, 0, &mut buf),
                    format!("queued {want} × {name}"),
                ))
            }
        };
        if let Some((encoded, said)) = encoded {
            match encoded {
                Ok(len) => match net.session.send_action(&buf[..len]) {
                    Ok(()) => ui.say(said),
                    Err(e) => ui.say(e.to_string()),
                },
                Err(e) => ui.say(format!("that would not encode ({e:?})")),
            }
        }
    }

    for (interaction, cat) in cats.iter() {
        if *interaction == Interaction::Pressed {
            if let Some(c) = RAIL.get(cat.0) {
                ui.cat = *c;
                ui.browser_scroll = 0.0;
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
            // And in its own look: a skin is per item, so the last pick's
            // would not fit this one.
            ui.skin = 0;
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
        match protocol::encode_action_craft(recipe, count, ui.skin, &mut buf) {
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

    for (interaction, chip) in chips.iter() {
        if *interaction == Interaction::Pressed {
            ui.skin = chip.0;
            ui.dirty = true;
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

/// A craft the sim refused, said on the page's status line. It reaches the
/// HUD's toast too (`hud::feedback`), but an open page covers the HUD, and
/// a click that is answered where the player is not looking is the
/// dark-panel defect.
pub fn sync_status(feed: Res<crate::render::feed::Feed>, mut ui: ResMut<Ui>) {
    if !matches!(ui.panel, Panel::Craft | Panel::Inventory) {
        return;
    }
    for (which, code, _item) in feed.refusals() {
        if which == crate::render::feed::Refused::Craft {
            ui.say(crate::ui::refusals::craft(code));
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
    mut ui: ResMut<Ui>,
    mut wheel: MessageReader<MouseWheel>,
    mut grids: Query<(&mut ScrollPosition, &ComputedNode), With<BrowserScroll>>,
) {
    if ui.panel != Panel::Craft {
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
        // Remembered, so the next redraw puts the grid back here.
        ui.browser_scroll = pos.0.y;
    }
}

/// How many recipes a player may star. A list fed by clicks is a list that
/// needs a cap (wall 4); the number itself is not a knob worth a row.
pub const MAX_FAVS: usize = 32;
