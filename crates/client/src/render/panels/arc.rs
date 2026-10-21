//! The arc's two screens (`ARC.md` F4):
//!
//! - **WORK** — `E` at a work's terminal, Rust's monument card: its three
//!   steps (sealed, open, burning) with the one it is on lit, one sentence on
//!   what to do now, what it unlocks for everybody (the things it lets anyone
//!   make or buy, and what its tank boosts), then its quota line by line with
//!   a picture, a bar, what is still owed and a DEPOSIT, and its tank.
//! - **ISLAND** — `O` anywhere: the act, every work's bar, what each one
//!   gives the whole server, and what the island holds now.
//!
//! Both draw from `ClientCore::arc` and send one `ACT_ARC` per press; reach,
//! state and what you carry are the sim's verdict, and its answer lands on
//! the status line ([`sync_status`]). Words come from `crate::ui::arc`.

use bevy::prelude::*;

use super::kit::{self, BAR_FILL, BAR_TANK};
use super::{Panel, Ui, BADGE, CELL_BG, CELL_FULL, LINE_HOT, TEXT, TEXT_DIM, TEXT_SHORT};
use crate::render::feed::{Feed, Refused};
use crate::render::icons::Icons;
use crate::ui::arc as words;
use crate::ui::craft::item_label;
use client_core::arc::WorkView;
use client_core::core::ClientCore;
use sim_core::works::{ARG_ALL, NO_UNLOCK, OP_DEPOSIT, OP_EXTRACT, OP_FUEL, WORK_LIT, WORK_OPEN};

const WORK_W: f32 = 680.0;
const ISLAND_W: f32 = 640.0;
/// A standings board's column, and the gap between two: two to a row.
const BOARD_W: f32 = 296.0;
const BOARD_GAP: f32 = 12.0;
/// A quota line's picture, px.
const ICON_PX: f32 = 34.0;
/// The label on the right of the work's own bar.
const BAR_LABEL_W: f32 = 80.0;
/// How many of an unlock's items its card pictures, and names.
const UNLOCK_ICONS: usize = 8;
const UNLOCK_NAMES: usize = 4;
/// A step behind the work, and an unlock the island holds: the badge's
/// green, dark.
const STEP_DONE: Color = Color::srgba(0.20, 0.27, 0.13, 0.95);

/// A press on a work: `ACT_ARC`'s op, target and argument.
#[derive(Component, Clone, Copy)]
pub struct ArcButton {
    pub op: u8,
    pub target: u8,
    pub arg: u8,
}

/// The server tick this client believes it is, for the hours a work names.
fn now(core: &ClientCore) -> f64 {
    core.clock.server_est
}

pub fn build_work(commands: &mut Commands, ui: &Ui, core: &ClientCore, icons: &Icons) {
    let k = ui.work as usize;
    let Some(w) = core.arc.works.get(k).filter(|w| w.known).copied() else {
        kit::screen(commands, WORK_W, |p| {
            kit::header(p, "THE WORK", None, &ui.status);
            kit::line(p, "it has not woken for you yet", 13.0, TEXT_DIM);
            kit::hint(p, "[ESC] CLOSE");
        });
        return;
    };
    let have = |item: u16| sim_core::craft::inv_count(&core.inv, item);
    kit::screen(commands, WORK_W, |p| {
        kit::header(p, w.name(), Some(words::act_title(w.def.act)), &ui.status);
        steps(p, &w);
        kit::line(p, words::todo_line(&w, now(core)), 13.0, TEXT);
        match w.state {
            WORK_LIT if w.def.fuel_max > 0 => labelled_bar(
                p,
                words::tank_pct(&w) * 10,
                BAR_TANK,
                format!("TANK {}%", words::tank_pct(&w)),
            ),
            WORK_LIT => {}
            _ => labelled_bar(
                p,
                w.progress_pm(),
                BAR_FILL,
                format!("{}% FILLED", w.progress_pm() / 10),
            ),
        }

        unlocks(p, core, icons, &w);

        let open = w.state == WORK_OPEN;
        if w.state != WORK_LIT {
            let any = w
                .def
                .inputs
                .iter()
                .enumerate()
                .take(w.def.n_inputs as usize)
                .any(|(i, input)| have(input.item) > 0 && w.got[i] < input.need);
            section_with(p, "WHAT IT NEEDS", |r| {
                kit::button(
                    r,
                    "DEPOSIT ALL",
                    ArcButton {
                        op: OP_DEPOSIT,
                        target: k as u8,
                        arg: ARG_ALL,
                    },
                    open && any,
                );
            });
        } else {
            kit::section(p, "WHAT IT TOOK");
        }
        for (i, input) in w
            .def
            .inputs
            .iter()
            .enumerate()
            .take(w.def.n_inputs as usize)
        {
            let got = w.got[i].min(input.need);
            let carry = have(input.item);
            let left = input.need - got;
            let pm = (got as u64 * 1000 / input.need.max(1) as u64) as u32;
            kit::row(p, |r| {
                icon(r, core, icons, input.item, ICON_PX);
                name_bar(r, item_label(&core.catalog, input.item), pm, BAR_FILL);
                kit::cell(r, words::quota_count(got, input.need), 110.0, TEXT);
                kit::cell(
                    r,
                    words::quota_left(got, input.need),
                    86.0,
                    if left == 0 { BADGE } else { TEXT_DIM },
                );
                kit::cell(
                    r,
                    format!("you have {}", words::thousands(carry)),
                    96.0,
                    if carry > 0 && left > 0 {
                        TEXT
                    } else {
                        TEXT_DIM
                    },
                );
                if w.state != WORK_LIT {
                    kit::button(
                        r,
                        "DEPOSIT",
                        ArcButton {
                            op: OP_DEPOSIT,
                            target: k as u8,
                            arg: i as u8,
                        },
                        open && carry > 0 && left > 0,
                    );
                }
            });
        }

        if w.def.fuel_max > 0 {
            let carry = have(w.def.fuel);
            let lit = w.state == WORK_LIT;
            kit::section(p, "THE TANK");
            kit::row(p, |r| {
                icon(r, core, icons, w.def.fuel, ICON_PX);
                name_bar(
                    r,
                    item_label(&core.catalog, w.def.fuel),
                    words::tank_pct(&w) * 10,
                    BAR_TANK,
                );
                kit::cell(r, words::quota_count(w.fuel, w.def.fuel_max), 110.0, TEXT);
                kit::cell(r, format!("-{} / h", w.def.burn_per_hour), 86.0, TEXT_DIM);
                kit::cell(
                    r,
                    format!("you have {}", words::thousands(carry)),
                    96.0,
                    if carry > 0 { TEXT } else { TEXT_DIM },
                );
                kit::button(
                    r,
                    "FUEL",
                    ArcButton {
                        op: OP_FUEL,
                        target: k as u8,
                        arg: 0,
                    },
                    lit && carry > 0 && w.fuel < w.def.fuel_max,
                );
            });
            if !lit {
                kit::line(p, "the tank takes fuel once it burns", 11.0, TEXT_DIM);
            }
        }
        if core.bank.work as usize == k {
            // THE EXCHANGE: carried JUNK leaves for the wallet here.
            let bank = core.bank;
            let carry = have(bank.coin);
            let (state, mine) = words::exit_lines(&bank);
            kit::section(p, "THE WAY OUT");
            kit::strong(p, state, 13.0, TEXT);
            kit::row(p, |r| {
                icon(r, core, icons, bank.coin, ICON_PX);
                kit::cell(
                    r,
                    item_label(&core.catalog, bank.coin).to_uppercase(),
                    180.0,
                    TEXT,
                );
                kit::cell(r, format!("you have {carry}"), 110.0, TEXT_DIM);
                kit::cell(r, words::exit_room(&bank), 182.0, TEXT_DIM);
                kit::button(
                    r,
                    "EXTRACT",
                    ArcButton {
                        op: OP_EXTRACT,
                        target: k as u8,
                        arg: 0,
                    },
                    words::can_extract(&bank, carry),
                );
            });
            if !mine.is_empty() {
                kit::line(p, mine, 11.0, LINE_HOT);
            }
        }
        if let Some(share) = words::share_line(&w) {
            kit::strong(p, share, 12.0, LINE_HOT);
        }
        kit::hint(p, "[ESC] CLOSE");
    });
}

/// The work's own bar, with what it measures on its right.
fn labelled_bar(p: &mut ChildSpawnerCommands, pm: u32, fill: Color, label: String) {
    kit::row(p, |r| {
        kit::bar(r, pm, fill, Some(WORK_W - 28.0 - 10.0 - BAR_LABEL_W));
        r.spawn((
            Node {
                width: Val::Px(BAR_LABEL_W),
                justify_content: JustifyContent::FlexEnd,
                ..default()
            },
            Pickable::IGNORE,
        ))
        .with_children(|c| {
            c.spawn((Text::new(label), super::font_bold(12.0), TextColor(TEXT)));
        });
    });
}

/// An item's picture, `px` square, or its name where there is none.
fn icon(r: &mut ChildSpawnerCommands, core: &ClientCore, icons: &Icons, item: u16, px: f32) {
    let name = item_label(&core.catalog, item);
    let mut tile = r.spawn((
        Node {
            width: Val::Px(px),
            height: Val::Px(px),
            flex_shrink: 0.0,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        },
        BackgroundColor(CELL_BG),
        Pickable::IGNORE,
    ));
    match icons.item(&name) {
        Some(h) => {
            tile.insert(ImageNode {
                color: crate::render::icons::PICTURE,
                ..ImageNode::new(h)
            });
        }
        None => {
            let initial: String = name.chars().take(2).collect::<String>().to_uppercase();
            tile.with_children(|t| {
                t.spawn((
                    Text::new(initial),
                    super::font_bold(px * 0.38),
                    TextColor(TEXT_DIM),
                    Pickable::IGNORE,
                ));
            });
        }
    }
}

/// A quota line's left half: the item's name over its own bar.
fn name_bar(r: &mut ChildSpawnerCommands, name: String, pm: u32, fill: Color) {
    r.spawn((
        Node {
            width: Val::Px(180.0),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(4.0),
            ..default()
        },
        Pickable::IGNORE,
    ))
    .with_children(|c| {
        c.spawn((
            Text::new(name.to_uppercase()),
            super::font_bold(13.0),
            TextColor(TEXT),
            Pickable::IGNORE,
        ));
        kit::bar(c, pm, fill, Some(180.0));
    });
}

/// A section rule with something on its right: a button.
fn section_with(
    p: &mut ChildSpawnerCommands,
    title: &str,
    right: impl FnOnce(&mut ChildSpawnerCommands),
) {
    p.spawn((
        Node {
            margin: UiRect::top(Val::Px(6.0)),
            padding: UiRect::bottom(Val::Px(3.0)),
            border: UiRect::bottom(Val::Px(1.0)),
            justify_content: JustifyContent::SpaceBetween,
            align_items: AlignItems::FlexEnd,
            ..default()
        },
        BorderColor::all(super::LINE),
    ))
    .with_children(|s| {
        s.spawn((
            Text::new(title),
            super::font_bold(11.0),
            TextColor(TEXT_DIM),
        ));
        right(s);
    });
}

/// The three steps, Rust's monument progression: the ones behind in the
/// badge's green, the one it is on lit, the one ahead dim.
fn steps(p: &mut ChildSpawnerCommands, w: &WorkView) {
    let (at, last) = words::step(w);
    p.spawn(Node {
        flex_direction: FlexDirection::Row,
        align_items: AlignItems::Center,
        margin: UiRect::vertical(Val::Px(4.0)),
        ..default()
    })
    .with_children(|r| {
        for (i, label) in words::STEPS.iter().enumerate() {
            let label = if i == 2 { last } else { label };
            let (bg, border, ink) = match i.cmp(&at) {
                std::cmp::Ordering::Less => (STEP_DONE, BADGE, BADGE),
                std::cmp::Ordering::Equal if last == "EMBERS" && i == 2 => {
                    (CELL_FULL, TEXT_SHORT, TEXT_SHORT)
                }
                std::cmp::Ordering::Equal => (CELL_FULL, LINE_HOT, LINE_HOT),
                std::cmp::Ordering::Greater => (CELL_BG, super::LINE, TEXT_DIM),
            };
            if i > 0 {
                r.spawn((
                    Node {
                        width: Val::Px(36.0),
                        height: Val::Px(2.0),
                        ..default()
                    },
                    BackgroundColor(if i <= at { BADGE } else { super::LINE }),
                ));
            }
            r.spawn((
                Node {
                    padding: UiRect::axes(Val::Px(12.0), Val::Px(5.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    column_gap: Val::Px(6.0),
                    align_items: AlignItems::Center,
                    ..default()
                },
                BackgroundColor(bg),
                BorderColor::all(border),
            ))
            .with_children(|c| {
                c.spawn((
                    Text::new(format!("{}", i + 1)),
                    super::font_bold(11.0),
                    TextColor(TEXT_DIM),
                ));
                c.spawn((Text::new(label), super::font_bold(13.0), TextColor(ink)));
            });
        }
    });
}

/// What the work gives everybody, as two cards side by side: what its
/// burning unlocks for the rest of the wipe (and the things that lets
/// anyone make or buy), and what its tank does while it holds fuel.
fn unlocks(p: &mut ChildSpawnerCommands, core: &ClientCore, icons: &Icons, w: &WorkView) {
    let floor = (w.def.floor != NO_UNLOCK).then(|| w.floor_name());
    let ceiling = (w.def.ceiling != NO_UNLOCK).then(|| w.ceiling_name());
    if floor.is_none() && ceiling.is_none() {
        return;
    }
    kit::section(p, "WHAT IT UNLOCKS FOR EVERYONE");
    p.spawn(Node {
        flex_direction: FlexDirection::Row,
        column_gap: Val::Px(10.0),
        ..default()
    })
    .with_children(|r| {
        if let Some(name) = floor {
            let held = core.arc.holds(w.def.floor);
            let items = words::unlocked_items(w.def.floor, &core.recipes, &core.vend);
            unlock_card(r, name, "FOR THE WIPE", held, ("UNLOCKED", "LOCKED"), |c| {
                if items.is_empty() {
                    return;
                }
                c.spawn(Node {
                    flex_direction: FlexDirection::Row,
                    column_gap: Val::Px(4.0),
                    ..default()
                })
                .with_children(|row| {
                    for &item in items.iter().take(UNLOCK_ICONS) {
                        icon(row, core, icons, item, 28.0);
                    }
                });
                let mut names: Vec<String> = items
                    .iter()
                    .take(UNLOCK_NAMES)
                    .map(|&i| item_label(&core.catalog, i))
                    .collect();
                if items.len() > UNLOCK_NAMES {
                    names.push(format!("+{} more", items.len() - UNLOCK_NAMES));
                }
                kit::line(
                    c,
                    format!("make or buy {}", names.join(", ")),
                    11.0,
                    TEXT_DIM,
                );
            });
        }
        if let Some(name) = ceiling {
            let on = core.arc.holds(w.def.ceiling);
            let boost = words::boost_line(w);
            unlock_card(r, name, "WHILE IT BURNS", on, ("ON", "OFF"), |c| {
                if let Some(b) = boost {
                    kit::strong(c, b, 13.0, if on { LINE_HOT } else { TEXT });
                }
                if w.def.fuel_max > 0 {
                    let fuel = item_label(&core.catalog, w.def.fuel);
                    kit::line(c, format!("needs {fuel} in its tank"), 11.0, TEXT_DIM);
                }
            });
        }
    });
}

/// One of [`unlocks`]' cards: its name, how long it lasts, whether the
/// island has it, and `body`.
fn unlock_card(
    r: &mut ChildSpawnerCommands,
    name: &str,
    lasts: &str,
    held: bool,
    (yes, no): (&str, &str),
    body: impl FnOnce(&mut ChildSpawnerCommands),
) {
    r.spawn((
        Node {
            flex_grow: 1.0,
            flex_basis: Val::Px(0.0),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(4.0),
            padding: UiRect::all(Val::Px(8.0)),
            border: UiRect::all(Val::Px(1.0)),
            ..default()
        },
        BackgroundColor(if held { STEP_DONE } else { CELL_BG }),
        BorderColor::all(if held { BADGE } else { super::LINE }),
    ))
    .with_children(|c| {
        c.spawn(Node {
            justify_content: JustifyContent::SpaceBetween,
            ..default()
        })
        .with_children(|h| {
            h.spawn((
                Text::new(lasts),
                super::font_bold(10.0),
                TextColor(TEXT_DIM),
            ));
            h.spawn((
                Text::new(if held { yes } else { no }),
                super::font_bold(10.0),
                TextColor(if held { BADGE } else { TEXT_DIM }),
            ));
        });
        c.spawn((Text::new(name), super::font_bold(16.0), TextColor(TEXT)));
        body(c);
    });
}

/// An ISLAND tab: 0 the works, 1 the journal, 2 the standings.
#[derive(Component, Clone, Copy)]
pub struct IslandTab(pub u8);

pub fn build_island(
    commands: &mut Commands,
    ui: &Ui,
    core: &ClientCore,
    friends: Option<&crate::friends::FriendSet>,
) {
    let arc = &core.arc;
    kit::page(commands, ui, ISLAND_W, |p| {
        kit::header(
            p,
            "THE ISLAND",
            Some(words::act_title(arc.act())),
            &ui.status,
        );
        kit::row(p, |r| {
            kit::button(r, "THE WORKS", IslandTab(0), ui.island_tab == 0);
            kit::button(r, "STANDINGS", IslandTab(2), ui.island_tab == 2);
            kit::button(r, "JOURNAL", IslandTab(1), ui.island_tab == 1);
        });
        match ui.island_tab {
            1 => journal(p, core),
            2 => standings(p, core, friends),
            _ => works(p, core),
        }
        kit::hint(p, "[O] OR [ESC] CLOSE");
    });
}

fn works(p: &mut ChildSpawnerCommands, core: &ClientCore) {
    let arc = &core.arc;
    kit::line(
        p,
        "The island is broken. The whole server switches it back on, \
         and what one work gives, it gives everybody.",
        12.0,
        TEXT_DIM,
    );
    let mut any = false;
    for (_, w) in arc.known() {
        any = true;
        kit::section(p, w.name());
        kit::strong(p, words::state_line(w, now(core)), 13.0, TEXT);
        match w.state {
            WORK_LIT if w.def.fuel_max > 0 => kit::bar(p, words::tank_pct(w) * 10, BAR_TANK, None),
            WORK_LIT => {}
            _ => kit::bar(p, w.progress_pm(), BAR_FILL, None),
        }
        let gives = words::gives_line(w);
        if !gives.is_empty() {
            kit::line(p, format!("gives everyone  {gives}"), 12.0, LINE_HOT);
        }
        if let Some(share) = words::share_line(w) {
            kit::line(p, share, 11.0, TEXT_DIM);
        }
    }
    if !any {
        kit::line(p, "no work has woken on this island yet", 13.0, TEXT_DIM);
    }
    kit::section(p, "THE ISLAND HOLDS");
    let mut held: Vec<&str> = Vec::new();
    for (_, w) in arc.known() {
        if w.def.floor != 0 && arc.holds(w.def.floor) {
            held.push(w.floor_name());
        }
        if w.def.ceiling != 0 && arc.holds(w.def.ceiling) {
            held.push(w.ceiling_name());
        }
    }
    if held.is_empty() {
        kit::line(p, "nothing yet — it starts broken", 12.0, TEXT_DIM);
    } else {
        kit::strong(p, held.join("  ·  "), 13.0, TEXT);
    }
}

/// The wipe's standings: four boards and the last wipe's island, each with
/// its top rows and where this player stands (`SUB_STANDING`).
fn standings(
    p: &mut ChildSpawnerCommands,
    core: &ClientCore,
    friends: Option<&crate::friends::FriendSet>,
) {
    kit::line(
        p,
        "Ranked all wipe. What you give the works is yours for good; what \
         your base holds counts only if it is still yours when the wipe lands. \
         Where a board has a purse, its top places are paid to the wallet you \
         play with after the wipe. Click a name to see their profile.",
        12.0,
        TEXT_DIM,
    );
    let me = core
        .tag(core.player_id)
        .map(|t| t.address)
        .filter(|a| !a.is_guest());
    let last = &core.standings[protocol::STANDING_BOARDS as usize - 1];
    // Two boards a row, so all four fit a 720p screen without scrolling.
    p.spawn(Node {
        flex_direction: FlexDirection::Row,
        flex_wrap: FlexWrap::Wrap,
        column_gap: Val::Px(BOARD_GAP),
        row_gap: Val::Px(4.0),
        ..default()
    })
    .with_children(|grid| {
        for (board, b) in core.standings.iter().enumerate() {
            let board = board as u8;
            if board == protocol::STANDING_BOARDS - 1 && last.n == 0 {
                continue;
            }
            grid.spawn(Node {
                width: Val::Px(BOARD_W),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(3.0),
                ..default()
            })
            .with_children(|p| board_column(p, b, board, me, friends));
        }
    });
}

/// One board: its name and what it ranks, its purse, its top rows and where
/// this player stands.
fn board_column(
    p: &mut ChildSpawnerCommands,
    b: &protocol::StandingBoard,
    board: u8,
    me: Option<protocol::Address>,
    friends: Option<&crate::friends::FriendSet>,
) {
    kit::section(p, &words::board_heading(b, board));
    kit::line(p, words::board_blurb(board), 11.0, TEXT_DIM);
    if let Some(purse) = words::purse_line(b) {
        kit::strong(p, purse, 12.0, TEXT);
    }
    if b.n == 0 {
        kit::line(p, "nobody yet — the board is open", 12.0, TEXT_DIM);
    }
    for i in 0..b.n as usize {
        let wallet = b.wallet(i);
        let mine = wallet.is_some() && wallet == me;
        let ink = match wallet {
            _ if mine => LINE_HOT,
            Some(a) if friends.is_some_and(|f| f.has(&a)) => crate::render::ui::FRIEND,
            _ => TEXT,
        };
        standing_row(p, wallet, b.name(i), |r| {
            kit::cell(r, format!("#{}", i + 1), 26.0, TEXT_DIM);
            kit::cell(
                r,
                if mine {
                    format!("{} (you)", b.name(i))
                } else {
                    b.name(i).to_string()
                },
                168.0,
                ink,
            );
            kit::cell(r, words::board_score(board, b.scores[i]), 76.0, TEXT);
        });
    }
    kit::strong(p, words::standing_you(b), 12.0, LINE_HOT);
    if let Some(prize) = words::prize_you(b) {
        kit::line(p, prize, 11.0, LINE_HOT);
    }
}

/// One standings row: a button to the player's profile card when the row
/// names a wallet, a plain row when it does not (a guest).
fn standing_row(
    p: &mut ChildSpawnerCommands,
    wallet: Option<protocol::Address>,
    name: &str,
    body: impl FnOnce(&mut ChildSpawnerCommands),
) {
    let mut row = p.spawn(Node {
        flex_direction: FlexDirection::Row,
        align_items: AlignItems::Center,
        column_gap: Val::Px(6.0),
        padding: UiRect::axes(Val::Px(4.0), Val::Px(1.0)),
        ..default()
    });
    if let Some(wallet) = wallet {
        let rest = Color::NONE;
        row.insert((
            Button,
            super::profile::ProfileOpen {
                wallet,
                name: name.to_string(),
            },
            BackgroundColor(rest),
            super::Hover::on(rest),
        ));
    }
    row.with_children(body);
}

/// What this player has learned: the glyphs, and the stones read.
fn journal(p: &mut ChildSpawnerCommands, core: &ClientCore) {
    let lore = &*core.lore;
    kit::section(p, "THE GLYPHS");
    kit::line(p, super::lore::reading_line(lore), 12.0, TEXT_DIM);
    let alphabet = core::str::from_utf8(lore.alphabet()).unwrap_or("");
    // The whole alphabet, spaced: what you read shows its letter.
    let spaced: String = alphabet.chars().flat_map(|c| [c, ' ']).collect();
    super::lore::glyph_text(p, spaced.trim_end(), lore);
    kit::section(p, "STONES YOU HAVE READ");
    let mut any = false;
    for (_, ins) in lore.known_inscriptions().filter(|(_, i)| i.len > 0) {
        any = true;
        super::lore::glyph_text(p, ins.text(), lore);
    }
    if !any {
        kit::line(p, "none yet — the old stones are out there", 12.0, TEXT_DIM);
    }
    let locks: Vec<&str> = lore.known_mechs().map(|(_, m)| m.name()).collect();
    if !locks.is_empty() {
        kit::section(p, "LOCKS ON THE ISLAND");
        kit::line(p, locks.join("  ·  "), 12.0, TEXT);
    }
}

/// A tab press switches the island page.
pub fn tabs(mut ui: ResMut<Ui>, presses: Query<(&Interaction, &IslandTab), Changed<Interaction>>) {
    if ui.panel != Panel::Island {
        return;
    }
    for (interaction, t) in presses.iter() {
        if *interaction == Interaction::Pressed && ui.island_tab != t.0 {
            ui.island_tab = t.0;
            ui.dirty = true;
        }
    }
}

/// A press sends the verb; the work's bars and the status line answer.
pub fn clicks(
    mut ui: ResMut<Ui>,
    net: NonSend<super::super::Net>,
    presses: Query<(&Interaction, &ArcButton), Changed<Interaction>>,
) {
    if ui.panel != Panel::Work {
        return;
    }
    for (interaction, b) in presses.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        let mut buf = [0u8; protocol::MAX_STREAM_MSG_BYTES];
        match protocol::encode_action_arc(b.op, b.target, b.arg, &mut buf) {
            Ok(len) => match net.session.send_action(&buf[..len]) {
                Ok(()) => ui.say(match b.op {
                    OP_FUEL => "feeding it…",
                    OP_EXTRACT => "putting it through the exchange…",
                    _ => "giving it what you carry…",
                }),
                Err(e) => ui.say(e.to_string()),
            },
            Err(e) => ui.say(format!("that would not encode ({e:?})")),
        }
    }
}

/// The status line hears the sim: a refusal, or the work changing.
pub fn sync_status(feed: Res<Feed>, mut ui: ResMut<Ui>, net: NonSend<super::super::Net>) {
    if !matches!(ui.panel, Panel::Work | Panel::Island) {
        return;
    }
    for (which, code, _) in feed.refusals() {
        if which == Refused::Arc {
            ui.say(crate::ui::refusals::arc(code));
        }
    }
    let core = &net.session.core;
    for &(index, what, _) in feed.work_events() {
        if ui.panel == Panel::Island || index == ui.work {
            if let Some(line) = words::event_line(&core.arc, index, what, None) {
                ui.say(line);
            }
        }
    }
}

/// Whether the open work is still in reach — the panel shuts when it is not,
/// the tree's posture (`panels::keys`).
pub fn work_in_reach(core: &ClientCore, k: u8) -> bool {
    let [x, y, z] = core.predict.position();
    core.arc.works.get(k as usize).is_some_and(|w| {
        w.known
            && sim_core::spot::within(
                core.haven(),
                &w.def.spot,
                x,
                y,
                z,
                sim_core::works::WORK_REACH_M + 1.0,
            )
    })
}
