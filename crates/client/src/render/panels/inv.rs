//! The inventory screen: your slots, the open container's, and the drag
//! between them.
//!
//! The whole screen is one tree — crafting on the left, the detail pane on
//! the right, the queue under them, your inventory and the container along
//! the bottom — because that is what the reference `inventory.jpeg` is. A
//! player pulls something out of a box and crafts with it without closing
//! anything.
//!
//! ## The gestures, and why they are three and not one
//!
//! | gesture | what it sends |
//! |---|---|
//! | left-drag | the whole stack |
//! | right-drag | half, rounded up |
//! | ctrl-left-drag | one unit |
//! | right-click, no drag | use the item (`ACT_CONSUME`) |
//!
//! All four are the same wire verb with a different `count`, except the
//! last, which falls out for free: a right-press released on the slot it
//! started on is a move to its own address, [`crate::ui::slots::move_args`]
//! refuses exactly that, and the refusal is the branch. So the gesture set
//! costs one `if` rather than a mode.
//!
//! ## The three regions are titled, and the container's says what it is
//!
//! The reference loot frame is three named blocks — the crafting tab across
//! the top, `INVENTORY`, and `LOOT` over a bar naming the thing you opened
//! (`LARGE WOODEN BOX`). Ours drew a screen-wide `INVENTORY` title and then
//! labelled the two grids `YOU` and `BOX`, which is the same information
//! arranged so that the word `INVENTORY` appears twice and the container is
//! named after its category rather than itself.
//!
//! So the screen title now names the region it actually sits over — the top
//! half of this screen *is* the crafting UI — the two grids carry their own
//! heads, and the container gets a name bar
//! (`crate::ui::slots::container_name`, resolved out of the deploy sync the
//! client is already drawing, so no byte of wire is owed for it).
//!
//! **It costs 3 px of the 720p column**, which is worth stating because
//! `CELL_PX`'s budget is what decides such things and the obvious reading is
//! that three new headings cost three headings' height. They do not: the
//! screen title was renamed, not added, so the only growth is the two
//! section heads going 12 px → 15 px, and the name bar sits on the
//! **container** panel — two rows where yours is five — so the row is as
//! tall as your grid either way and the bar is free.
//!
//! ## What the panel is not allowed to do
//!
//! It does not move anything. A drag draws a ghost and sends a request; the
//! cells redraw from `ClientCore`'s view when the sim answers. There is no
//! optimistic copy of a container anywhere in this file, because the
//! reference's worst failure on this verb is a container state that diverged
//! from the server's — and it presented as *the player being disconnected*.
//!
//! That is also why the drag's source cell only *lights up* rather than
//! emptying. Drawing the source as empty while the tile rides the cursor is
//! the prettier lie, and it is a one-cell optimistic copy of a container —
//! exactly the thing this file refuses everywhere else.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use client_core::core::ClientCore;
use sim_core::gather::ItemStack;
use sim_core::inventory::{CONT_SELF, REFUSE_M_MAX};
use sim_core::limits::{HOTBAR_SLOTS, INV_SLOTS};

use super::{
    craft, font, font_bold, GhostRoot, Panel, PanelRoot, Ui, BADGE, CELL_BG, CELL_FULL,
    CELL_GAP_PX, CELL_HOVER, CELL_PX, LINE, LINE_HOT, PANEL_BG, SCRIM, TEXT, TEXT_DIM,
};
use crate::render::icons::Icons;
use crate::ui::craft::{cell_abbrev, item_label, CELL_LINE_CHARS};
use crate::ui::slots::{
    container_cols, container_name, count_badge, ghost_origin, move_args, refusal_text, slots_in,
    Drag, Grab,
};

/// One addressable cell. `kind` is a `CONT_*`, so the same component serves
/// your inventory and whatever container is open — the drag code never asks
/// which grid it is looking at, which is what makes cross-container drags
/// fall out rather than being a second path.
#[derive(Component, Clone, Copy)]
pub struct SlotCell {
    pub kind: u8,
    pub slot: usize,
}

/// Build the whole screen.
pub fn build_screen(commands: &mut Commands, ui: &Ui, core: &ClientCore, icons: &Icons) {
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
            header(root, ui);

            // The upper half: crafting, then the detail pane.
            root.spawn(Node {
                flex_direction: FlexDirection::Row,
                column_gap: Val::Px(8.0),
                ..default()
            })
            .with_children(|row| {
                craft::build_browser(row, ui, core, icons);
                craft::build_detail(row, ui, core);
            });

            craft::build_queue(root, ui, core);

            // The lower half: your slots, and the container if one is open.
            root.spawn(Node {
                flex_direction: FlexDirection::Row,
                column_gap: Val::Px(8.0),
                ..default()
            })
            .with_children(|row| {
                own_grid(row, core, icons);
                if core.cont_kind != CONT_SELF {
                    container_grid(row, core, icons);
                }
            });

            // Names only the two closers, deliberately. `I` and `Q` open this
            // screen and cannot close it — the search box eats every letter,
            // so a letter that closed would close it mid-word — and a hint
            // promising a round trip that does not exist is worse than a hint
            // that is merely incomplete.
            root.spawn((
                Text::new(
                    "drag to move   -   right-drag takes half   -   ctrl-drag takes one   \
                     -   right-click uses   -   Tab or Esc closes",
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

fn header(root: &mut ChildSpawnerCommands, ui: &Ui) {
    // **`CRAFTING`, not `INVENTORY`.** It sits directly over the recipe
    // browser, the detail pane and the queue, which are what the top half of
    // this screen is; the inventory has its own head on the grid it names,
    // and a title that labels the region under it can only be one of the two.
    root.spawn((Text::new("CRAFTING"), font_bold(26.0), TextColor(TEXT)));
    // Always drawn, even empty: the line's job is to have somewhere to say
    // why something did not happen, and a line that appears and disappears
    // makes the panel jump when it does.
    root.spawn((
        Text::new(if ui.status.is_empty() {
            " ".into()
        } else {
            ui.status.clone()
        }),
        font(13.0),
        TextColor(BADGE),
    ));
}

/// Your own 30 slots: the belt on its own row, then the grid.
fn own_grid(row: &mut ChildSpawnerCommands, core: &ClientCore, icons: &Icons) {
    row.spawn((
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
        section(col, "INVENTORY");
        // The belt is drawn apart from the grid because it is apart: it is
        // the row the world can see, and slot 0..6 is what `sel` indexes.
        grid(col, core, icons, CONT_SELF, 0, HOTBAR_SLOTS, HOTBAR_SLOTS);
        col.spawn((
            Node {
                width: Val::Percent(100.0),
                height: Val::Px(1.0),
                ..default()
            },
            BackgroundColor(LINE),
        ));
        grid(
            col,
            core,
            icons,
            CONT_SELF,
            HOTBAR_SLOTS,
            INV_SLOTS,
            HOTBAR_SLOTS,
        );
    });
}

/// The open container. Its width is the container's own, not the view's:
/// the wire ships `INV_SLOTS` slots whatever kind is open and the tail stays
/// zero for a box, so a panel that drew all thirty would draw twelve slots
/// and eighteen lies.
fn container_grid(row: &mut ChildSpawnerCommands, core: &ClientCore, icons: &Icons) {
    let kind = core.cont_kind;
    let n = slots_in(kind);
    let name = container_name(
        kind,
        core.cont_handle,
        core.deploys.entries(),
        &core.deploy_defs,
        core.deploy_defs_have,
        &core.catalog,
    );
    row.spawn((
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
        section(col, "LOOT");
        name_bar(col, &name);
        grid(col, core, icons, kind, 0, n, container_cols(kind));
    });
}

/// The strip under `LOOT` naming what was opened. Spans the grid because it
/// is a heading for the whole grid — a name floated over the left cell reads
/// as a label for that cell.
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

/// `slots` of `kind`, `cols` wide, drawn from the core's view of it.
#[allow(clippy::too_many_arguments)]
fn grid(
    parent: &mut ChildSpawnerCommands,
    core: &ClientCore,
    icons: &Icons,
    kind: u8,
    from: usize,
    to: usize,
    cols: usize,
) {
    let view: &[ItemStack; INV_SLOTS] = if kind == CONT_SELF {
        &core.inv
    } else {
        &core.cont
    };
    parent
        .spawn(Node {
            display: Display::Grid,
            grid_template_columns: RepeatedGridTrack::px(cols as u16, CELL_PX),
            row_gap: Val::Px(CELL_GAP_PX),
            column_gap: Val::Px(CELL_GAP_PX),
            ..default()
        })
        .with_children(|g| {
            for (slot, stack) in view.iter().enumerate().take(to).skip(from) {
                cell(g, kind, slot, *stack, core, icons);
            }
        });
}

fn cell(
    parent: &mut ChildSpawnerCommands,
    kind: u8,
    slot: usize,
    stack: ItemStack,
    core: &ClientCore,
    icons: &Icons,
) {
    let filled = stack.count > 0;
    parent
        .spawn((
            Button,
            SlotCell { kind, slot },
            Node {
                width: Val::Px(CELL_PX),
                height: Val::Px(CELL_PX),
                border: UiRect::all(Val::Px(1.0)),
                padding: UiRect::all(Val::Px(3.0)),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::SpaceBetween,
                overflow: Overflow::clip(),
                ..default()
            },
            BackgroundColor(if filled { CELL_FULL } else { CELL_BG }),
            BorderColor::all(LINE),
        ))
        .with_children(|c| {
            if filled {
                // **A picture, not a word.** `Gunpowde` and `Workbenc` are
                // what a 44 px cell does to an item name, and every cell in
                // `crafting.png` is artwork. The name still exists — the
                // detail pane and the drag ghost print it — so the cell can
                // afford to be the thing rather than the label for it.
                let name = item_label(&core.catalog, stack.item);
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
                            color: Color::srgb(0.90, 0.87, 0.82),
                            ..default()
                        },
                        Pickable::IGNORE,
                    )),
                    // No icon baked for this item: the word, as before —
                    // abbreviated to the cell (`ui::craft::cell_abbrev`),
                    // because the clip alone cuts mid-glyph and `Gunpowde`
                    // reads as a defect. An empty cell would be the
                    // dark-panel defect.
                    None => c.spawn((
                        Text::new(cell_abbrev(&name, CELL_LINE_CHARS)),
                        font_bold(10.0),
                        TextColor(TEXT),
                        Pickable::IGNORE,
                    )),
                };
                // A count of one is not drawn, and the count that is carries
                // its `x` — both are the reference frame's own rules, and
                // `count_badge` is where they are written down and tested.
                if let Some(badge) = count_badge(stack.count) {
                    c.spawn((
                        Text::new(badge),
                        font_bold(13.0),
                        TextColor(TEXT),
                        Node {
                            align_self: AlignSelf::FlexEnd,
                            ..default()
                        },
                        Pickable::IGNORE,
                    ));
                }
            }
        });
}

/// A region head — `INVENTORY`, `LOOT`. Full `TEXT` at 15 px, not the dim
/// 12 px these were: they are the only titles this screen has now, and a
/// heading in the same weight and colour as the hint line under the grid is
/// not a heading.
fn section(parent: &mut ChildSpawnerCommands, text: &str) {
    parent.spawn((
        Text::new(text.to_string()),
        font_bold(15.0),
        TextColor(TEXT),
    ));
}

/// Press, drag, release. The only system that sends a move.
#[allow(clippy::too_many_arguments)]
pub fn drag_pointer(
    mut commands: Commands,
    mut ui: ResMut<Ui>,
    net: NonSend<super::super::Net>,
    mouse: Res<ButtonInput<MouseButton>>,
    keyboard: Res<ButtonInput<KeyCode>>,
    window: Query<&Window, With<PrimaryWindow>>,
    mut cells: Query<(
        &SlotCell,
        &Interaction,
        &mut BorderColor,
        &mut BackgroundColor,
    )>,
    ghosts: Query<Entity, With<GhostRoot>>,
    // `Option` for `rebuild`'s reason: `icons::load` is a `Startup` system,
    // and a ghost asked for before it has run draws the label it drew before
    // rather than an empty tile.
    icons: Option<Res<super::super::icons::Icons>>,
) {
    if ui.panel != Panel::Inventory {
        if ui.drag.is_some() {
            ui.drag = None;
            for e in ghosts.iter() {
                commands.entity(e).despawn();
            }
        }
        return;
    }

    let core = &net.session.core;

    // Which cell the pointer is over. `Pressed` counts as over: Bevy flips a
    // hovered node to `Pressed` while the button is down, and a drag that
    // stopped tracking its own source the moment the button went down would
    // never find a target.
    //
    // The fill is set here rather than at build time for the same reason the
    // border always was: hover changes every frame and a rebuild per frame is
    // what `Seen` exists to prevent. Which means the *resting* fill has to be
    // recomputed here too — it is a function of the core's view, not of what
    // the cell was last painted, so a cell that empties while the pointer
    // rests on it comes back to `CELL_BG` and not to whatever it was.
    let mut over: Option<SlotCell> = None;
    for (cell, interaction, mut border, mut bg) in cells.iter_mut() {
        let hot = !matches!(interaction, Interaction::None);
        if hot {
            over = Some(*cell);
        }
        let source = ui
            .drag
            .map(|d| d.kind == cell.kind && d.slot == cell.slot)
            .unwrap_or(false);
        let want = if hot || source { LINE_HOT } else { LINE };
        if border.top != want {
            *border = BorderColor::all(want);
        }
        let view = if cell.kind == CONT_SELF {
            &core.inv
        } else {
            &core.cont
        };
        let filled = view[cell.slot.min(INV_SLOTS - 1)].count > 0;
        let fill = match (hot || source, filled) {
            (true, _) => CELL_HOVER,
            (false, true) => CELL_FULL,
            (false, false) => CELL_BG,
        };
        if bg.0 != fill {
            *bg = BackgroundColor(fill);
        }
    }

    // ---- press ----------------------------------------------------------
    if ui.drag.is_none() {
        let grab = if mouse.just_pressed(MouseButton::Right) {
            Some(Grab::Half)
        } else if mouse.just_pressed(MouseButton::Left) {
            Some(
                if keyboard.pressed(KeyCode::ControlLeft) || keyboard.pressed(KeyCode::ControlRight)
                {
                    Grab::One
                } else {
                    Grab::All
                },
            )
        } else {
            None
        };
        if let (Some(grab), Some(cell)) = (grab, over) {
            let view = if cell.kind == CONT_SELF {
                &core.inv
            } else {
                &core.cont
            };
            let stack = view[cell.slot.min(INV_SLOTS - 1)];
            if stack.count > 0 {
                ui.drag = Some(Drag {
                    kind: cell.kind,
                    slot: cell.slot,
                    grab,
                    stack,
                });
                // Placed at the cursor here as well as by `ghost_follow`,
                // and that is not belt-and-braces: a spawn is deferred to
                // the end of the schedule, so a tile that waited to be
                // positioned would be drawn once at the window's top-left
                // corner before its first follow — a flash in the corner
                // every time a drag starts.
                let at = window
                    .single()
                    .ok()
                    .and_then(|w| w.cursor_position())
                    .map(|p| ghost_origin(p.x, p.y, CELL_PX))
                    .unwrap_or((0.0, 0.0));
                let fallback = super::super::icons::Icons::default();
                let icons = icons.as_deref().unwrap_or(&fallback);
                spawn_ghost(&mut commands, core, icons, stack, grab, at);
            }
        }
        return;
    }

    // ---- release --------------------------------------------------------
    if mouse.pressed(MouseButton::Left) || mouse.pressed(MouseButton::Right) {
        return;
    }
    let drag = ui.drag.take().expect("checked above");
    for e in ghosts.iter() {
        commands.entity(e).despawn();
    }

    let Some(target) = over else {
        // Released over the world. Nothing is sent — dropping an item on the
        // ground is a verb the sim does not have yet (`MENUS.md` §4), and
        // inventing one here would be the client deciding.
        ui.say("released over nothing - the item stayed put");
        return;
    };

    // Right-press, released where it started: use the item. Falls out of the
    // move refusing its own address rather than being a separate mode.
    if target.kind == drag.kind && target.slot == drag.slot {
        if drag.grab == Grab::Half && drag.kind == CONT_SELF {
            use_item(&mut ui, &net, drag.slot);
        }
        return;
    }

    let Some(args) = move_args(
        core.cont_handle,
        drag.kind,
        drag.slot,
        target.kind,
        target.slot,
        drag.grab,
        &core.inv,
        &core.cont,
    ) else {
        // The refusals this side owns are all "the panel cannot address
        // that", and every one of them would otherwise cost a round trip and
        // a rolled-back prediction — see `ui::slots`.
        ui.say("that move cannot be addressed from here");
        return;
    };

    let mut buf = [0u8; protocol::MAX_STREAM_MSG_BYTES];
    match args.encode(&mut buf) {
        Ok(len) => match net.session.send_action(&buf[..len]) {
            Ok(()) => ui.status.clear(),
            Err(e) => ui.say(e.to_string()),
        },
        // An encoder refusal is a client bug, not a player one, and it is
        // reported rather than swallowed: a bad action frame ends the reader
        // task server-side, so refusing to encode keeps the bug local to the
        // panel instead of arriving as a disconnect.
        Err(e) => ui.say(format!("the move would not encode ({e:?})")),
    }
}

/// `ACT_CONSUME` for an inventory slot. Whether the item is food, and
/// whether eating it does anything, are the sim's verdict.
fn use_item(ui: &mut Ui, net: &super::super::Net, slot: usize) {
    let Ok(slot) = u8::try_from(slot) else {
        return;
    };
    let mut buf = [0u8; protocol::MAX_STREAM_MSG_BYTES];
    match protocol::encode_action_consume(slot, &mut buf) {
        Ok(len) => match net.session.send_action(&buf[..len]) {
            Ok(()) => ui.status.clear(),
            Err(e) => ui.say(e.to_string()),
        },
        Err(e) => ui.say(format!("that cannot be used ({e:?})")),
    }
}

/// The thing in your hand: **a cell, under the cursor**.
///
/// It was a text pill reading `Wood x8`, hung off the pointer's lower right.
/// Two things were wrong with that and only one is cosmetic. The pill did
/// not look like the thing it came out of — every reference frame this panel
/// is measured against carries the item's own picture on the cursor, which
/// is what makes a drag legible at a glance across two grids. And an
/// *offset* payload is aimed by parallax: the cell the drop addresses is the
/// one under the pointer, and drawing the cargo somewhere else asks the
/// player to hold that correction in their head for the length of the drag.
///
/// So the ghost is a copy of the source cell — same edge, same fill, same
/// icon, same badge — sitting centred on the pointer, with the name captioned
/// under it because a silhouette is a picture of a category and the name is
/// the identity. `units` on the badge, not `stack.count`: a right-drag takes
/// half, and the tile must say what will land rather than what was there.
fn spawn_ghost(
    commands: &mut Commands,
    core: &ClientCore,
    icons: &Icons,
    stack: ItemStack,
    grab: Grab,
    at: (f32, f32),
) {
    let units = grab.units(stack.count);
    let name = item_label(&core.catalog, stack.item);
    commands
        .spawn((
            GhostRoot,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(at.0),
                top: Val::Px(at.1),
                width: Val::Px(CELL_PX),
                height: Val::Px(CELL_PX),
                padding: UiRect::all(Val::Px(3.0)),
                border: UiRect::all(Val::Px(1.0)),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::SpaceBetween,
                ..default()
            },
            BackgroundColor(CELL_HOVER),
            BorderColor::all(LINE_HOT),
            // The thing in your hand must never eat a pointer event — it is
            // under the cursor by definition, so a pickable ghost would be
            // the only thing the drag could ever release onto. Every node of
            // it, not just this one: `Pickable` is per-entity, so a picked
            // child would swallow the drop the root was written to let past.
            Pickable::IGNORE,
            GlobalZIndex(50),
        ))
        .with_children(|g| {
            match icons.item(&name) {
                Some(image) => g.spawn((
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
                        color: Color::srgb(0.90, 0.87, 0.82),
                        ..default()
                    },
                    Pickable::IGNORE,
                )),
                // No icon baked: the abbreviated word the cell falls back to,
                // so the ghost and the cell it came out of never disagree
                // about what an item looks like.
                None => g.spawn((
                    Text::new(cell_abbrev(&name, CELL_LINE_CHARS)),
                    font_bold(10.0),
                    TextColor(TEXT),
                    Pickable::IGNORE,
                )),
            };
            if let Some(badge) = count_badge(units) {
                g.spawn((
                    Text::new(badge),
                    font_bold(13.0),
                    TextColor(TEXT),
                    Node {
                        align_self: AlignSelf::FlexEnd,
                        ..default()
                    },
                    Pickable::IGNORE,
                ));
            }
            // Hung off the tile rather than laid out under it: an in-flow
            // caption would be as wide as the name and would recentre the
            // tile it is captioning, which is the one thing this ghost's
            // position is not allowed to depend on.
            g.spawn((
                Text::new(name),
                font(11.0),
                TextColor(TEXT_DIM),
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(0.0),
                    top: Val::Px(CELL_PX),
                    ..default()
                },
                Pickable::IGNORE,
            ));
        });
}

/// Keep the ghost centred on the cursor. The arithmetic is
/// [`ghost_origin`]'s, which is where the reason lives.
pub fn ghost_follow(
    window: Query<&Window, With<PrimaryWindow>>,
    mut ghosts: Query<&mut Node, With<GhostRoot>>,
) {
    let Ok(window) = window.single() else {
        return;
    };
    let Some(p) = window.cursor_position() else {
        return;
    };
    let (x, y) = ghost_origin(p.x, p.y, CELL_PX);
    for mut node in ghosts.iter_mut() {
        node.left = Val::Px(x);
        node.top = Val::Px(y);
    }
}

/// Turn the sim's last move refusal into the status line. Read once per
/// change: `last_move_refused` is a latch, not a queue.
pub fn note_refusal(ui: &mut Ui, reason: u8) {
    if reason > 0 && reason as u32 <= REFUSE_M_MAX {
        ui.say(refusal_text(reason));
    }
}
