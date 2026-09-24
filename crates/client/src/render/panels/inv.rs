//! The inventory screen: your slots, the open container's, and the drag
//! between them.
//!
//! The whole screen is one tree, and **what is in it depends on whether a
//! container is open.** With nothing open it is the crafting screen —
//! the recipe browser, the detail pane, the queue — over your slots and
//! your body. With a container open the crafting half is not drawn at all
//! and the container's grid takes the room (the operator, 2026-09-16:
//! *"we shouldnt show crafting"*).
//!
//! ⚠ **That is a reversal, and the paragraph it replaced was the argument
//! for the other side**: *"a player pulls something out of a box and
//! crafts with it without closing anything"*, measured against the
//! reference `inventory.jpeg`. It was a fair reading of a frame with no
//! loot panel in it — the reference's *loot* frame puts the container
//! where the crafting tab was, so the thing we copied from a screenshot
//! was the half of their layout that does not survive opening a box. The
//! cost is real and stated: to craft you close the container, because this
//! screen has no tab strip (`NOW.md` §0p2).
//!
//! ## The gestures, and why they are three and not one
//!
//! | gesture | what it sends |
//! |---|---|
//! | left-drag | the whole stack |
//! | right-drag | half, rounded up |
//! | ctrl-left-drag | one unit |
//! | right-click, no drag, nothing open | use the item (`ACT_CONSUME`) |
//! | right-click, no drag, container open | move it across ([`crate::ui::slots::quick_move`]) |
//!
//! All four drags are the same wire verb with a different `count`. The
//! fifth row is the same verb again and it is the one the operator asked
//! for — *"right clicking should put it into ur inventory u dont have to
//! drag"* — and what makes it a quick-move rather than a second path is
//! that the panel picks the destination slot and then marshals it through
//! `move_args` like any drag. Both no-drag rows fall out of one `if`: a
//! right-press released on the slot it started on is a move to its own
//! address, which `move_args` refuses, and that refusal is the branch.
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
use sim_core::combat::ARMOR_MAX_PCT;
use sim_core::gather::ItemStack;
use sim_core::inventory::{CONT_BOX, CONT_SELF, CONT_WEAR, REFUSE_M_BUSY, REFUSE_M_MAX};
use sim_core::limits::{HOTBAR_SLOTS, INV_SLOTS, WEAR_SLOTS};
use sim_core::research::{TABLE_COIN_SLOT, TABLE_ITEM_SLOT};

use super::{
    craft, font, font_bold, GhostRoot, Panel, PanelRoot, Ui, BADGE, CELL_BG, CELL_FULL,
    CELL_GAP_PX, CELL_HOVER, CELL_PX, CELL_SEL, LINE, LINE_HOT, PANEL_BG, PAPER_BG, PIP_FILL,
    PIP_H_PX, PIP_TROUGH, SCRIM, TEXT, TEXT_DIM, TEXT_SHORT,
};
use crate::render::icons::Icons;
use crate::ui::craft::{cell_abbrev, item_label, CELL_LINE_CHARS};
use crate::ui::research::{self as research_ui, TableLine, UseAs};
use crate::ui::slots::{
    container_bar, container_cols, container_name, container_title, count_badge, ghost_origin,
    looting, move_args, pip_fraction, quick_move, refusal_text, screen_title, slots_in,
    takes_deposits, wear_slot_label, wearable_here, worn_pct, Drag, Grab, Quick,
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
pub fn build_screen(commands: &mut Commands, ui: &Ui, core: &ClientCore, icons: &Icons, sel: u8) {
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
            header(root, ui, core);

            // The upper half: crafting, then the detail pane — **and not
            // while a container is open** (the operator, 2026-09-16:
            // *"we shouldnt show crafting"*, looking at the recipe
            // browser drawn over a bag's slots).
            //
            // The reference's loot frame is the same shape: the panel you
            // opened takes the screen and crafting is behind a tab. We
            // have no tab, so the crafting half is reachable by closing
            // the container — which is the cost, and `NOW.md` §0p2
            // carries it rather than this file inventing a tab strip.
            //
            // It is a `build_screen` branch rather than a `Visibility`
            // toggle because `panels::rebuild` tears this tree down and
            // respawns it on every change: a hidden browser would still
            // cost its ~40 cells of nodes and would still be the thing
            // `ui.query`'s keystrokes went to.
            if !looting(core.cont_kind) {
                root.spawn(Node {
                    flex_direction: FlexDirection::Row,
                    column_gap: Val::Px(8.0),
                    ..default()
                })
                .with_children(|row| {
                    craft::build_browser(row, ui, core, icons);
                    craft::build_detail(row, ui, core, icons);
                });

                craft::build_queue(root, ui, core, icons);
            }

            // The lower half: your slots, your body, and the container if
            // one is open.
            //
            // **The body is drawn unconditionally now** (`NOW.md` §0eq
            // item 4). It used to be a branch inside `container_grid`, so
            // it appeared only when `CONT_WEAR` was the open container —
            // and a box was the open container the whole time you were
            // looting one, which made the panel's own move (helmet out of
            // a raided box, onto a head) the one route it could not draw.
            // It is fed by its own stream now and is never absent, so the
            // three panels sit left-to-right in the order the move runs:
            // take from the box on the right, drop on the body in the
            // middle, or hold it in the pack on the left.
            root.spawn(Node {
                flex_direction: FlexDirection::Row,
                column_gap: Val::Px(8.0),
                ..default()
            })
            .with_children(|row| {
                own_grid(row, core, icons, sel as usize);
                wear_panel(row, core, icons);
                if open_table(core).is_some() {
                    table_grid(row, ui, core, icons);
                } else if looting(core.cont_kind) {
                    container_grid(row, core, icons);
                }
            });

            // Names only the two closers, deliberately. `I` and `Q` open this
            // screen and cannot close it — the search box eats every letter,
            // so a letter that closed would close it mid-word — and a hint
            // promising a round trip that does not exist is worse than a hint
            // that is merely incomplete.
            root.spawn((
                Text::new(if open_table(core).is_some() {
                    // The table's own gestures: which slot takes what, and
                    // what starts it — the two things no other container
                    // asks a player to know.
                    "one item to research in ITEM, junk beside it   -   right-click moves it across   \
                     -   BEGIN or C starts it   -   Tab or Esc closes"
                } else if looting(core.cont_kind) {
                    // What the gesture DOES here, which is not what it
                    // does with nothing open. A hint line that names the
                    // other mode is worse than no hint: it is a promise.
                    "drag to move   -   right-click moves it across   \
                     -   right-drag takes half   -   ctrl-drag takes one   \
                     -   Tab or Esc closes"
                } else {
                    "drag to move   -   right-drag takes half   -   ctrl-drag takes one   \
                     -   right-click uses   -   Tab or Esc closes"
                }),
                font(12.0),
                TextColor(TEXT_DIM),
                Node {
                    margin: UiRect::top(Val::Px(6.0)),
                    ..default()
                },
            ));

            super::spawn_tip(root);
        });
}

fn header(root: &mut ChildSpawnerCommands, ui: &Ui, core: &ClientCore) {
    // **`CRAFTING`, not `INVENTORY`.** It sits directly over the recipe
    // browser, the detail pane and the queue, which are what the top half of
    // this screen is; the inventory has its own head on the grid it names,
    // and a title that labels the region under it can only be one of the two.
    //
    // Which is exactly why it is not a constant any more: with a container
    // open there IS no recipe browser under it, so the word is
    // `ui::slots::screen_title`'s to choose and `LOOTING` is what it
    // chooses (`tests/ui.rs` §T).
    root.spawn((
        // A research table is not loot (research table v1): the screen is
        // where a research is started, and LOOTING over it would name a
        // verb the player is not doing.
        Text::new(if open_table(core).is_some() {
            "RESEARCH"
        } else {
            screen_title(core.cont_kind)
        }),
        font_bold(26.0),
        TextColor(TEXT),
    ));
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
fn own_grid(row: &mut ChildSpawnerCommands, core: &ClientCore, icons: &Icons, sel: usize) {
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
        grid(
            col,
            core,
            icons,
            CONT_SELF,
            0,
            HOTBAR_SLOTS,
            HOTBAR_SLOTS,
            sel,
        );
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
            sel,
        );
    });
}

/// The open container. Its width is the container's own, not the view's:
/// the wire ships `INV_SLOTS` slots whatever kind is open and the tail stays
/// zero for a box, so a panel that drew all thirty would draw twelve slots
/// and eighteen lies.
fn container_grid(row: &mut ChildSpawnerCommands, core: &ClientCore, icons: &Icons) {
    let kind = core.cont_kind;
    // The body is not loot, and it is not drawn from here. It had a
    // branch at the top of this function until 2026-08-28, when it got
    // its own stream and its own permanent column beside this one —
    // `core.cont_kind` can no longer be `CONT_WEAR` at all, because the
    // server refuses to open the body into the ground subscription.
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
        section(col, container_title(kind));
        if let Some(bar) = container_bar(kind, &name) {
            name_bar(col, bar);
        }
        grid(col, core, icons, kind, 0, n, container_cols(kind), NO_SEL);
    });
}

/// The open container's address, if what is open is a **research table**
/// (research table v1) — `ui::research::table_open`, read off the deploy
/// sync the client already draws.
fn open_table(core: &ClientCore) -> Option<(u16, u16, u8)> {
    research_ui::table_open(
        core.cont_kind,
        core.cont_handle,
        core.deploys.entries(),
        &core.deploy_defs,
        core.deploy_defs_have,
    )
    .then(|| research_ui::table_address(core.cont_handle))
}

/// Whether the open research table is running — the lit bit the core heard
/// for its address (`EV_OVEN`). `false` for anything else, which is what
/// `panels::detect_changes` needs: the table's slots do not move when a
/// research starts, only this does.
pub(crate) fn open_table_running(core: &ClientCore) -> bool {
    open_table(core).is_some_and(|(cx, cz, level)| core.ovens().is_lit(cx, cz, level))
}

/// The BEGIN button under a research table's slots.
#[derive(Component)]
pub struct TableBegin;

/// The wait bar's fill, whose width [`table_clock`] sets each frame.
#[derive(Component)]
pub struct TableFill;

/// Width of the table's column contents: its two captioned cells.
const TABLE_W_PX: f32 = 2.0 * CELL_PX + 64.0;

/// The research table, in the container's seat (research table v1): its
/// two working slots captioned by what goes in them, the line saying what
/// the table will do (`ui::research::table_line` — the sim's start check,
/// read on this side), the wait while it runs, and BEGIN.
///
/// The cells are `cell()`, unchanged, so the drag path does not know this
/// container is shaped differently — the slot indices are the sim's
/// (`research::TABLE_ITEM_SLOT`, `TABLE_COIN_SLOT`) and nothing else of the
/// box's twelve is drawn, because nothing else of it takes anything.
fn table_grid(row: &mut ChildSpawnerCommands, ui: &Ui, core: &ClientCore, icons: &Icons) {
    let running = open_table_running(core);
    let line = research_ui::table_line(&core.research, &core.cont, running);
    let coin = item_label(&core.catalog, core.research.coin);
    let name = container_name(
        CONT_BOX,
        core.cont_handle,
        core.deploys.entries(),
        &core.deploy_defs,
        core.deploy_defs_have,
        &core.catalog,
    );
    let ready = matches!(line, TableLine::Ready { .. });
    let good = matches!(line, TableLine::Ready { .. } | TableLine::Done { .. });
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
        // The table's own name as the head, and no name bar under it: the
        // bar exists to name a container its category cannot, and this
        // head already does — a bar would say RESEARCH TABLE twice.
        section(col, &name);
        col.spawn(Node {
            flex_direction: FlexDirection::Row,
            column_gap: Val::Px(12.0),
            ..default()
        })
        .with_children(|r| {
            for (slot, caption) in [
                (TABLE_ITEM_SLOT, "ITEM".to_string()),
                (TABLE_COIN_SLOT, coin.to_uppercase()),
            ] {
                r.spawn(Node {
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    row_gap: Val::Px(3.0),
                    ..default()
                })
                .with_children(|c| {
                    cell(
                        c,
                        CONT_BOX,
                        slot,
                        cell_stack(core, CONT_BOX, slot),
                        core,
                        icons,
                        false,
                    );
                    c.spawn((Text::new(caption), font(10.0), TextColor(TEXT_DIM)));
                });
            }
        });
        col.spawn((
            Text::new(research_ui::table_text(&core.catalog, line, &coin)),
            font(12.0),
            TextColor(if good { BADGE } else { TEXT_DIM }),
            Node {
                max_width: Val::Px(TABLE_W_PX + 60.0),
                ..default()
            },
        ));
        // The wait, only when this client saw it start — a bar begun at
        // zero on a table opened halfway through would be a bar that lies
        // (`ui::research::TableClock`).
        if running && ui.table_clock.timed() {
            col.spawn((
                Node {
                    width: Val::Px(TABLE_W_PX),
                    height: Val::Px(PIP_H_PX * 2.0),
                    ..default()
                },
                BackgroundColor(PIP_TROUGH),
            ))
            .with_children(|t| {
                t.spawn((
                    TableFill,
                    Node {
                        width: Val::Percent(0.0),
                        height: Val::Percent(100.0),
                        ..default()
                    },
                    BackgroundColor(BADGE),
                ));
            });
        }
        col.spawn((
            Button,
            TableBegin,
            Node {
                padding: UiRect::axes(Val::Px(14.0), Val::Px(4.0)),
                border: UiRect::all(Val::Px(1.0)),
                align_self: AlignSelf::FlexStart,
                ..default()
            },
            BackgroundColor(if ready { BADGE } else { CELL_BG }),
            BorderColor::all(if ready { BADGE } else { LINE }),
        ))
        .with_children(|b| {
            b.spawn((
                Text::new("BEGIN"),
                font_bold(13.0),
                TextColor(if ready {
                    Color::srgb(0.08, 0.08, 0.06)
                } else {
                    TEXT_DIM
                }),
                Pickable::IGNORE,
            ));
        });
    });
}

/// BEGIN: the table's switch, `ACT_USE` at its address — the same action
/// the `C` key sends at a fire. A press on a table that is not ready says
/// why instead of sending a request the sim would refuse with the same
/// sentence a round trip later.
pub fn table_clicks(
    mut ui: ResMut<Ui>,
    net: NonSend<super::super::Net>,
    begin: Query<&Interaction, (Changed<Interaction>, With<TableBegin>)>,
) {
    if ui.panel != Panel::Inventory || !begin.iter().any(|i| *i == Interaction::Pressed) {
        return;
    }
    let core = &net.session.core;
    let Some((cx, cz, level)) = open_table(core) else {
        return;
    };
    let line = research_ui::table_line(&core.research, &core.cont, open_table_running(core));
    if !matches!(line, TableLine::Ready { .. }) {
        let coin = item_label(&core.catalog, core.research.coin);
        ui.say(research_ui::table_text(&core.catalog, line, &coin).to_lowercase());
        return;
    }
    let mut buf = [0u8; protocol::MAX_STREAM_MSG_BYTES];
    match protocol::encode_action_use(cx, cz, level, sim_core::build::LOC_PLANE, &mut buf) {
        Ok(len) => match net.session.send_action(&buf[..len]) {
            Ok(()) => ui.status.clear(),
            Err(e) => ui.say(e.to_string()),
        },
        Err(e) => ui.say(format!("the table would not start ({e:?})")),
    }
}

/// The table's wait, every frame: feed the clock what the core says about
/// the open table, and set the bar's fill from it. Before `rebuild` in the
/// chain, so the frame a research starts is also the frame the panel is
/// rebuilt knowing it was seen starting.
pub fn table_clock(
    time: Res<Time>,
    mut ui: ResMut<Ui>,
    net: NonSend<super::super::Net>,
    mut fills: Query<&mut Node, With<TableFill>>,
) {
    let core = &net.session.core;
    let now = time.elapsed_secs();
    let (handle, running) = if ui.panel == Panel::Inventory && open_table(core).is_some() {
        (core.cont_handle, open_table_running(core))
    } else {
        (0, false)
    };
    let mut clock = ui.table_clock;
    clock.observe(handle, running, now);
    if clock != ui.table_clock {
        ui.table_clock = clock;
    }
    if let Some(f) = clock.fraction(now, core.research.table_ticks) {
        for mut node in fills.iter_mut() {
            node.width = Val::Percent(f * 100.0);
        }
    }
}

/// Height of the drawn silhouette's head block, and of its torso. Two
/// rectangles is the whole figure: `inventory.jpeg`'s paperdoll is a dim,
/// low-contrast body that exists to say *where* a slot sits on a person,
/// and at this size a detailed one would be noise. Sized so head + gap +
/// torso lines up against two 44 px cells and their captions.
const DOLL_HEAD_PX: f32 = 26.0;
const DOLL_TORSO_PX: f32 = 52.0;
const DOLL_W_PX: f32 = 40.0;

/// The worn container, drawn as a paperdoll rather than as a grid
/// (`NOW.md` §0eq.1, off `inventory.jpeg`).
///
/// Three things a two-cell grid could not say. **Which slot is which** —
/// the cells are captioned HEAD and BODY, in the sim's own slot order
/// (`combat::WEAR_HEAD` is 1 and the array is zero-based, so container
/// slot 0 is the head; the captions are derived from that mapping, not
/// listed twice). **That they are worn on a person** — the silhouette
/// beside them, which is what the reference frame uses the middle of the
/// panel for. **What the set is worth** — the protection line, which is
/// the whole reason wire v52 exists, because until this pass the client
/// held the worn items and no number to put beside them.
///
/// The cells themselves are `cell()`, unchanged, so the drag path does not
/// know this panel is shaped differently: `drag_pointer` finds a
/// `SlotCell { kind: CONT_WEAR, slot }` by query and neither knows nor
/// cares what is drawn around it.
fn wear_panel(row: &mut ChildSpawnerCommands, core: &ClientCore, icons: &Icons) {
    let pct = worn_pct(&core.catalog, &core.worn);
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
        section(col, container_title(CONT_WEAR));
        col.spawn(Node {
            flex_direction: FlexDirection::Row,
            column_gap: Val::Px(8.0),
            align_items: AlignItems::Center,
            ..default()
        })
        .with_children(|body| {
            body.spawn(Node {
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(CELL_GAP_PX),
                ..default()
            })
            .with_children(|stack| {
                for slot in 0..WEAR_SLOTS {
                    wear_slot(stack, core, icons, slot);
                }
            });
            silhouette(body);
        });
        protection_bar(col, pct);
    });
}

/// One captioned wear cell: the slot's name over the cell it addresses.
fn wear_slot(parent: &mut ChildSpawnerCommands, core: &ClientCore, icons: &Icons, slot: usize) {
    parent
        .spawn(Node {
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(2.0),
            ..default()
        })
        .with_children(|c| {
            c.spawn((
                Text::new(wear_slot_label(slot).to_string()),
                font(10.0),
                TextColor(TEXT_DIM),
                Pickable::IGNORE,
            ));
            cell(
                c,
                CONT_WEAR,
                slot,
                cell_stack(core, CONT_WEAR, slot),
                core,
                icons,
                false,
            );
        });
}

/// Head over torso, in the panel's own line colour at low contrast. Not an
/// asset: two `Node`s cost nothing, carry no licence and cannot go missing
/// from `assets/` — and `assets/models/WANTED.md` has no paperdoll row to
/// wait on.
fn silhouette(parent: &mut ChildSpawnerCommands) {
    parent
        .spawn(Node {
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            row_gap: Val::Px(3.0),
            ..default()
        })
        .with_children(|d| {
            // `border_radius` is a field on `Node` in this Bevy, not a
            // component beside it — `map.rs` and `wheel.rs` both spell it
            // this way, and spelling it the other way is a compile error
            // rather than the silent duplicate-component panic the trap
            // list warns about, which is the good half of the same rule.
            d.spawn((
                Node {
                    width: Val::Px(DOLL_HEAD_PX),
                    height: Val::Px(DOLL_HEAD_PX),
                    border_radius: BorderRadius::MAX,
                    ..default()
                },
                BackgroundColor(CELL_FULL),
                Pickable::IGNORE,
            ));
            d.spawn((
                Node {
                    width: Val::Px(DOLL_W_PX),
                    height: Val::Px(DOLL_TORSO_PX),
                    border_radius: BorderRadius::all(Val::Px(6.0)),
                    ..default()
                },
                BackgroundColor(CELL_FULL),
                Pickable::IGNORE,
            ));
        });
}

/// What the set is worth, as a word and a bar.
///
/// The bar's full width is `combat::ARMOR_MAX_PCT` and not 100, because 90
/// is the most the sim will ever subtract — a bar that filled to 100 would
/// read as "there is more armor to find" at the point where there is not.
fn protection_bar(parent: &mut ChildSpawnerCommands, pct: u32) {
    parent
        .spawn(Node {
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(3.0),
            ..default()
        })
        .with_children(|c| {
            c.spawn((
                Text::new(format!("PROTECTION {pct}%")),
                font_bold(11.0),
                TextColor(if pct > 0 { TEXT } else { TEXT_DIM }),
                Pickable::IGNORE,
            ));
            c.spawn((
                Node {
                    width: Val::Percent(100.0),
                    height: Val::Px(PIP_H_PX),
                    ..default()
                },
                BackgroundColor(PIP_TROUGH),
                Pickable::IGNORE,
            ))
            .with_children(|trough| {
                let frac = pct as f32 / ARMOR_MAX_PCT as f32;
                trough.spawn((
                    Node {
                        width: Val::Percent(frac * 100.0),
                        height: Val::Percent(100.0),
                        ..default()
                    },
                    BackgroundColor(PIP_FILL),
                    Pickable::IGNORE,
                ));
            });
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
/// **The stack a cell draws from, and the only place the view is
/// picked.** Three containers reach this panel — the pack, the body and
/// whatever is open on the ground — and until 2026-08-28 there were two,
/// so the pick was `if kind == CONT_SELF { inv } else { cont }` written
/// out at four call sites. Adding the third view to three of four is a
/// silent defect of exactly the shape the trap list names: the wrong
/// container answers with a *plausible* stack, so the panel draws, the
/// drag starts, and the count shipped to the server is some other item's.
///
/// `get` rather than an index, and no clamp. The three sites this
/// replaces each ended `[slot.min(INV_SLOTS - 1)]`, which turns an
/// out-of-range slot into slot 29's contents — a lie that draws. An
/// empty stack is the honest answer and the one a cell already knows how
/// to render.
fn cell_stack(core: &ClientCore, kind: u8, slot: usize) -> ItemStack {
    let view: &[ItemStack] = match kind {
        CONT_SELF => &core.inv,
        CONT_WEAR => &core.worn,
        _ => &core.cont,
    };
    view.get(slot).copied().unwrap_or_default()
}

#[allow(clippy::too_many_arguments)]
fn grid(
    parent: &mut ChildSpawnerCommands,
    core: &ClientCore,
    icons: &Icons,
    kind: u8,
    from: usize,
    to: usize,
    cols: usize,
    sel: usize,
) {
    parent
        .spawn(Node {
            display: Display::Grid,
            grid_template_columns: RepeatedGridTrack::px(cols as u16, CELL_PX),
            row_gap: Val::Px(CELL_GAP_PX),
            column_gap: Val::Px(CELL_GAP_PX),
            ..default()
        })
        .with_children(|g| {
            for slot in from..to {
                let stack = cell_stack(core, kind, slot);
                cell(
                    g,
                    kind,
                    slot,
                    stack,
                    core,
                    icons,
                    kind == CONT_SELF && slot == sel,
                );
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
    active: bool,
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
            BackgroundColor(resting_fill(core, stack, active)),
            BorderColor::all(LINE),
        ))
        // Its name under the pointer (`panels::tooltip`).
        .insert_if(
            super::Tip(research_ui::stack_label(
                &core.catalog,
                &core.research,
                stack,
            )),
            || filled,
        )
        .with_children(|c| {
            if filled {
                // **A picture, not a word.** `Gunpowde` and `Workbenc` are
                // what a 44 px cell does to an item name, and every cell in
                // `crafting.png` is artwork. The name still exists — the
                // detail pane and the drag ghost print it — so the cell can
                // afford to be the thing rather than the label for it.
                //
                // A blueprint is drawn as the thing it TEACHES, on the
                // paper's blue (research table v1): twelve sheets are twelve
                // pictures, not twelve identical scrolls.
                let name = research_ui::icon_name(&core.catalog, &core.research, stack);
                // A skinned item's icon wears its skin's tint (skins v0), and
                // a swatch in the corner says it is skinned even where the
                // tint is subtle.
                let tint = crate::ui::skins::tint_of(&core.skins, stack.skin);
                let [tr, tg, tb] = tint.unwrap_or([1.0, 1.0, 1.0]);
                if let Some([r, g, b]) = tint {
                    c.spawn((
                        Node {
                            position_type: PositionType::Absolute,
                            right: Val::Px(3.0),
                            top: Val::Px(3.0),
                            width: Val::Px(7.0),
                            height: Val::Px(7.0),
                            ..default()
                        },
                        BackgroundColor(Color::srgb(r, g, b)),
                        Pickable::IGNORE,
                    ));
                }
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
                        // A colour picture, drawn as it is — or through the
                        // skin's tint, the one colour a picture is
                        // multiplied by.
                        ImageNode {
                            image,
                            color: Color::srgb(tr, tg, tb),
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
                // Bottom right and shadowed: a colour picture is busier than
                // the white glyph it replaced, and a count laid over one has
                // to stand off whatever it lands on.
                if let Some(badge) = count_badge(stack.count) {
                    c.spawn(count_node(badge));
                }
                // The other number a cell carries, and the one a player has
                // no other way to read: `cond` has been on the wire since
                // v42 and its ceiling since v46, and until now every panel
                // drew both of them nowhere.
                pip(c, stack, core);
            }
        });
}

/// No slot of this grid is in the player's hand.
const NO_SEL: usize = usize::MAX;

/// What a cell rests on when nothing is pointing at it: the belt slot in
/// your hand in Rust's blue (the HUD's own mark for it, so the two agree),
/// a blueprint on its paper, a stack on the filled grey, nothing on the
/// empty one. One function for the build and for `drag_pointer`, which
/// repaints it every frame: two copies of this match were two answers.
fn resting_fill(core: &ClientCore, stack: ItemStack, active: bool) -> Color {
    if active {
        CELL_SEL
    } else if stack.count == 0 {
        CELL_BG
    } else if research_ui::is_paper(&core.research, stack) {
        PAPER_BG
    } else {
        CELL_FULL
    }
}

/// A cell's stack count: Rust's corner, bottom right, shadowed off the
/// picture under it. The cell and the drag ghost draw the same one.
fn count_node(badge: String) -> impl Bundle {
    (
        Text::new(badge),
        font_bold(12.0),
        TextColor(TEXT),
        crate::render::ui::TEXT_SHADOW,
        Node {
            position_type: PositionType::Absolute,
            right: Val::Px(2.0),
            bottom: Val::Px(PIP_H_PX),
            ..default()
        },
        Pickable::IGNORE,
    )
}

/// The durability pip: a thin trough under the cell's icon with a fill, or
/// nothing at all.
///
/// `ui::slots::pip_fraction` decides all three states and the argument for
/// them is written there — the short version is that a pristine tool draws
/// nothing, because a full bar under every fresh tool is a screen of bars.
///
/// **Absolute and pinned to both side edges** rather than laid out in the
/// cell's column: the flow in a cell is the icon and the count badge, and a
/// bar taking part in it would push the badge off the corner every reference
/// frame puts it in. Pinning `left` and `right` also means the bar is the
/// cell's own width whatever the padding and border do to the content box,
/// which is what the icon's hand-fitted `CELL_PX - 10` does not.
fn pip(parent: &mut ChildSpawnerCommands, stack: ItemStack, core: &ClientCore) {
    let Some(frac) = pip_fraction(stack.cond, core.catalog.cond_max(stack.item as usize)) else {
        return;
    };
    parent
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                bottom: Val::Px(0.0),
                height: Val::Px(PIP_H_PX),
                ..default()
            },
            BackgroundColor(PIP_TROUGH),
            Pickable::IGNORE,
        ))
        .with_children(|trough| {
            trough.spawn((
                Node {
                    width: Val::Percent(frac * 100.0),
                    height: Val::Percent(100.0),
                    ..default()
                },
                BackgroundColor(PIP_FILL),
                Pickable::IGNORE,
            ));
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

/// Skins in the inventory (skins v0): `P` over an item of yours cycles
/// it through the skins you own for it and back to its own look — Rust's
/// repair-bench skin picker, at any workbench since we have no repair bench
/// (`sim_core::skin::reskin`; the sim refuses away from one and says why).
///
/// And **opening the inventory asks the shard to read your skins again**
/// (`ACT_SKINS_REFRESH`), which is how a skin bought in the launcher reaches
/// the game: buy, come back, press Tab. The shard reads the platform at most
/// once per `server::skins::REFRESH_COOLDOWN`, so opening it often costs
/// nothing.
pub fn skin_keys(
    mut ui: ResMut<Ui>,
    net: NonSend<super::super::Net>,
    keyboard: Res<ButtonInput<KeyCode>>,
    cells: Query<(&SlotCell, &Interaction)>,
    mut was_open: Local<bool>,
) {
    let open = ui.panel == Panel::Inventory;
    if open && !*was_open {
        let mut buf = [0u8; protocol::MAX_STREAM_MSG_BYTES];
        if let Ok(len) = protocol::encode_action_skins_refresh(&mut buf) {
            // A full lane drops the ask; the next open asks again.
            let _ = net.session.send_action(&buf[..len]);
        }
    }
    *was_open = open;
    if !open || !keyboard.just_pressed(KeyCode::KeyP) {
        return;
    }
    let core = &net.session.core;
    let Some(cell) = cells
        .iter()
        .find(|(c, i)| c.kind == CONT_SELF && !matches!(i, Interaction::None))
        .map(|(c, _)| *c)
    else {
        // With the craft screen up, a `p` pointing at nothing is a letter in
        // the search box (`panels::keys`), not a request to say anything.
        if looting(core.cont_kind) {
            ui.say("point at an item in your inventory to change its skin");
        }
        return;
    };
    let stack = core.inv[cell.slot];
    if stack.count == 0 {
        return;
    }
    let next = crate::ui::skins::next_skin(&core.skins, &core.skins_owned, stack.item, stack.skin);
    if next == stack.skin {
        ui.say("you own no skins for this item - they are sold in the launcher's ITEM STORE");
        return;
    }
    let mut buf = [0u8; protocol::MAX_STREAM_MSG_BYTES];
    match protocol::encode_action_reskin(cell.slot as u8, next, &mut buf) {
        Ok(len) => match net.session.send_action(&buf[..len]) {
            Ok(()) => ui.say(match crate::ui::skins::name(&core.skins, next) {
                Some(name) => format!("skin: {name}"),
                None => "skin: default".to_string(),
            }),
            Err(e) => ui.say(e.to_string()),
        },
        Err(e) => ui.say(format!("that skin change would not encode ({e:?})")),
    }
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
        // **A wear slot says whether it will take this, before the
        // release.** The other four container kinds accept anything that
        // fits, so a border is only ever "you are over this cell"; the
        // body is the first one with an opinion about *what*, and until
        // wire v52 the client had no way to hold that opinion — the answer
        // came back as a refusal after the round trip, by which time the
        // prediction had already drawn the piece into the slot.
        //
        // Lit when it accepts (so a helmet picked up marks the head slot
        // across the panel), red only when the pointer is actually on a
        // slot that will refuse — a permanent red on every non-matching
        // slot would make dragging wood across the screen look like an
        // error.
        let takes = ui.drag.and_then(|d| {
            if cell.kind == CONT_WEAR && d.kind != CONT_WEAR {
                Some(wearable_here(&core.catalog, d.stack.item, cell.slot))
            } else if !takes_deposits(cell.kind) && d.kind != cell.kind {
                // The second kind with an opinion, and the cheapest: a
                // loot-only crate refuses every item, so the answer needs
                // no catalog and is always `false`. Drawn the same way —
                // red only under the pointer — so dragging *across* a
                // crate on the way to the pack is not a screen of errors.
                Some(false)
            } else {
                None
            }
        });
        let want = match takes {
            Some(true) => LINE_HOT,
            Some(false) if hot => TEXT_SHORT,
            _ if hot || source => LINE_HOT,
            _ => LINE,
        };
        if border.top != want {
            *border = BorderColor::all(want);
        }
        let resting = cell_stack(core, cell.kind, cell.slot);
        let active = cell.kind == CONT_SELF && cell.slot == net.sel as usize;
        let fill = if hot || source {
            CELL_HOVER
        } else {
            resting_fill(core, resting, active)
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
            let stack = cell_stack(core, cell.kind, cell.slot);
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

    // Right-press, released where it started: the **quick-move**, or the
    // use it has always been when nothing is open. Falls out of the move
    // refusing its own address rather than being a separate mode, exactly
    // as it did when `use` was the only thing it could mean.
    //
    // `Grab::Half` is the marker for "this was the right button" (the
    // press handler mints it), not a count — a quick-move measures its
    // own. Every branch of the decision is `ui::slots::quick_move`'s, so
    // this arm sends what it is handed and decides nothing.
    let table = open_table(core).is_some();
    if target.kind == drag.kind && target.slot == drag.slot {
        if drag.grab == Grab::Half {
            let quick = if table {
                research_ui::table_quick_move(
                    core.cont_handle,
                    open_table_running(core),
                    drag.kind,
                    drag.slot,
                    &core.catalog,
                    &core.research,
                    &core.inv,
                    &core.cont,
                    &core.worn,
                )
            } else {
                quick_move(
                    core.cont_kind,
                    core.cont_handle,
                    drag.kind,
                    drag.slot,
                    &core.catalog,
                    &core.inv,
                    &core.cont,
                    &core.worn,
                )
            };
            match quick {
                Quick::Send(args) => send_move(&mut ui, &net, args),
                Quick::Use(slot) => use_item(&mut ui, &net, slot),
                Quick::Refused(why) => ui.say(why),
            }
        }
        return;
    }

    // A loot-only container says the crate's own sentence rather than this
    // side's generic one. `move_args` refuses the move too (its step 6),
    // so this is about the WORDS: `refusal_text` is the sim's table, so
    // the line a player reads is identical whether the panel caught it or
    // the shard did.
    if sim_core::inventory::deposit_refused(
        drag.kind,
        target.kind,
        cell_stack(core, drag.kind, drag.slot),
        cell_stack(core, target.kind, target.slot),
    ) {
        ui.say(refusal_text(sim_core::inventory::REFUSE_M_NO_INPUT as u8));
        return;
    }

    // A research table has an opinion about every drop, and the obvious
    // ones are said here rather than after a round trip: the wrong thing
    // for a slot, or any move at all while it runs. A drop INTO the item
    // slot carries one unit whatever the button — the slot holds one.
    let mut grab = drag.grab;
    if table && (drag.kind == CONT_BOX || target.kind == CONT_BOX) {
        let running = open_table_running(core);
        if target.kind == CONT_BOX && drag.kind != CONT_BOX {
            if let Some(why) = research_ui::table_drop_refusal(
                &core.research,
                running,
                target.slot,
                drag.stack.item,
            ) {
                ui.say(why);
                return;
            }
            grab = research_ui::table_grab(target.slot == TABLE_ITEM_SLOT, grab);
        } else if running {
            ui.say(refusal_text(REFUSE_M_BUSY as u8));
            return;
        }
    }

    let Some(args) = move_args(
        core.cont_handle,
        drag.kind,
        drag.slot,
        target.kind,
        target.slot,
        grab,
        &core.inv,
        &core.cont,
        &core.worn,
    ) else {
        // The refusals this side owns are all "the panel cannot address
        // that", and every one of them would otherwise cost a round trip and
        // a rolled-back prediction — see `ui::slots`.
        ui.say("that move cannot be addressed from here");
        return;
    };

    send_move(&mut ui, &net, args);
}

/// Put a validated move on the wire. **One call site for `MoveArgs::encode`
/// in this file**, shared by the drag and the quick-move, which is the same
/// reason `MoveArgs` exists at all: the argument order is written down once
/// (in `ui::slots`) and a second marshalling path here would be a second
/// place to transpose two `u8`s.
fn send_move(ui: &mut Ui, net: &super::super::Net, args: crate::ui::slots::MoveArgs) {
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

/// Use an inventory slot: read it if it is a blueprint (`ACT_RESEARCH`,
/// research table v1), eat it otherwise (`ACT_CONSUME`). Which one is
/// `ui::research::use_as`'s call; whether it does anything is the sim's.
fn use_item(ui: &mut Ui, net: &super::super::Net, slot: usize) {
    let Ok(slot) = u8::try_from(slot) else {
        return;
    };
    let core = &net.session.core;
    let stack = core.inv.get(slot as usize).copied().unwrap_or_default();
    let mut buf = [0u8; protocol::MAX_STREAM_MSG_BYTES];
    let encoded = match research_ui::use_as(&core.research, stack) {
        UseAs::Read => protocol::encode_action_research(slot, &mut buf),
        UseAs::Consume => protocol::encode_action_consume(slot, &mut buf),
    };
    match encoded {
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
    // The caption is the stack's whole name ("Revolver Blueprint"); the
    // picture is the thing it teaches, as the cell it came out of drew it.
    let name = research_ui::stack_label(&core.catalog, &core.research, stack);
    let art = research_ui::icon_name(&core.catalog, &core.research, stack);
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
            match icons.item(&art) {
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
                        color: super::super::icons::PICTURE,
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
                g.spawn(count_node(badge));
            }
            // The ghost is a copy of the source cell, and the pip is part of
            // the copy: an item carrying condition never stacks (content rule
            // V7), so `units` is the whole of it and the bar on the cursor is
            // the bar that will land.
            pip(g, stack, core);
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
