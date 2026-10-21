//! CONTACTS (`Panel::Contacts`), Rust's: the third button in the strip over
//! the inventory and crafting pages. A grid of the players you have met, each
//! with the mugshot taken when you met them, the most recent first; pick one
//! and the pane on the right says when you met and lets you mark them
//! FRIENDLY or ENEMY, open their profile, or forget them.
//!
//! The book and its rules are `crate::contacts`; the watching and the grabs
//! are `render::contacts`. This file is nodes and presses.

use bevy::prelude::*;
use protocol::Address;

use super::kit;
use super::{
    font, font_bold, Hover, Panel, Ui, CELL_BG, CELL_FULL, LINE, LINE_HOT, TEXT, TEXT_DIM,
};
use crate::contacts::{self as book, Contact, Filter, Rel};
use crate::render::contacts::Contacts;
use crate::render::friends::{me_in_world, Friends};
use crate::render::profile::Profile;
use crate::render::screen::{Menu, Who};

const COLS: usize = 5;
const ROWS: usize = 3;
const PER_PAGE: usize = COLS * ROWS;
const CARD_PX: f32 = 96.0;
const GAP_PX: f32 = 6.0;
const GRID_W: f32 = COLS as f32 * CARD_PX + (COLS - 1) as f32 * GAP_PX;
const DETAIL_W: f32 = 300.0;
const SHOT_PX: f32 = 200.0;

/// Rust's nametag colours: friendly blue, enemy red.
pub const FRIENDLY: Color = Color::srgb(0.45, 0.66, 1.0);
pub const ENEMY: Color = Color::srgb(0.92, 0.36, 0.30);

/// A mark's colour: its own, or the plain ink for one only seen.
pub fn ink(rel: Rel) -> Color {
    match rel {
        Rel::Friendly => FRIENDLY,
        Rel::Enemy => ENEMY,
        Rel::Seen => TEXT,
    }
}

/// A press on the page.
#[derive(Component, Clone)]
pub enum ContactButton {
    Filter(Filter),
    Pick(String),
    Page(isize),
    Mark(Rel),
    Profile,
    Forget,
}

/// The picked contact, if it is in the list on show; else the first.
fn picked<'a>(ui: &Ui, shown: &[&'a Contact]) -> Option<&'a Contact> {
    ui.contacts_sel
        .as_deref()
        .and_then(|w| shown.iter().find(|c| c.wallet == w).copied())
        .or_else(|| shown.first().copied())
}

pub fn build_screen(
    commands: &mut Commands,
    ui: &Ui,
    contacts: Option<&Contacts>,
    friends: Option<&Friends>,
) {
    let fallback = Contacts::default();
    let contacts = contacts.unwrap_or(&fallback);
    let book = &contacts.book;
    let shown = book.shown(ui.contacts_filter);
    let pages = shown.len().div_ceil(PER_PAGE).max(1);
    let page = ui.contacts_page.min(pages - 1);
    let sel = picked(ui, &shown);
    let now = crate::shot::now_secs();
    let elo = |w: &str| {
        friends.is_some_and(|f| Address::from_hex(w.as_bytes()).is_some_and(|a| f.set.has(&a)))
    };
    super::page_root(commands, ui, |zone| {
        if book.contacts.is_empty() {
            zone.spawn(Node {
                width: Val::Px(GRID_W + DETAIL_W + 12.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: Val::Px(8.0),
                padding: UiRect::all(Val::Px(40.0)),
                ..default()
            })
            .with_children(|c| {
                kit::strong(c, "NOBODY YET", 18.0, TEXT);
                kit::line(
                    c,
                    "Spend a few seconds near another player and they land here, \
                     with a picture of the moment you met.",
                    13.0,
                    TEXT_DIM,
                );
            });
            hint(zone);
            return;
        }
        zone.spawn(Node {
            flex_direction: FlexDirection::Row,
            column_gap: Val::Px(12.0),
            ..default()
        })
        .with_children(|row| {
            row.spawn(Node {
                width: Val::Px(GRID_W),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(8.0),
                ..default()
            })
            .with_children(|col| {
                filters(col, ui, contacts, page, pages);
                grid(
                    col,
                    contacts,
                    &shown[(page * PER_PAGE).min(shown.len())..],
                    sel,
                    now,
                    &elo,
                );
            });
            detail(row, contacts, sel, now, &elo);
        });
        hint(zone);
    });
}

fn hint(zone: &mut ChildSpawnerCommands) {
    zone.spawn((
        Text::new(
            "a player you stand near for a few seconds is met   -   \
             whoever kills you is an enemy   -   Tab or Esc",
        ),
        font(12.0),
        TextColor(TEXT_DIM),
    ));
}

/// The filter row: ALL / FRIENDLY / ENEMY / SEEN with their counts, and the
/// page turner when there is more than one page.
fn filters(
    col: &mut ChildSpawnerCommands,
    ui: &Ui,
    contacts: &Contacts,
    page: usize,
    pages: usize,
) {
    col.spawn(Node {
        flex_direction: FlexDirection::Row,
        align_items: AlignItems::Center,
        column_gap: Val::Px(6.0),
        ..default()
    })
    .with_children(|r| {
        for f in Filter::ALL {
            let label = format!("{} {}", f.label(), contacts.book.count(f));
            kit::button(r, &label, ContactButton::Filter(f), ui.contacts_filter == f);
        }
        if pages > 1 {
            r.spawn(Node {
                flex_grow: 1.0,
                ..default()
            });
            kit::button(r, "PREV", ContactButton::Page(-1), page > 0);
            kit::line(r, format!("{} / {pages}", page + 1), 12.0, TEXT_DIM);
            kit::button(r, "NEXT", ContactButton::Page(1), page + 1 < pages);
        }
    });
}

fn grid(
    col: &mut ChildSpawnerCommands,
    contacts: &Contacts,
    from: &[&Contact],
    sel: Option<&Contact>,
    now: u64,
    elo: &dyn Fn(&str) -> bool,
) {
    col.spawn(Node {
        flex_direction: FlexDirection::Row,
        flex_wrap: FlexWrap::Wrap,
        column_gap: Val::Px(GAP_PX),
        row_gap: Val::Px(GAP_PX),
        width: Val::Px(GRID_W),
        ..default()
    })
    .with_children(|g| {
        for c in from.iter().take(PER_PAGE) {
            let on = sel.is_some_and(|s| s.wallet == c.wallet);
            let border = if on {
                LINE_HOT
            } else if c.rel == Rel::Seen {
                LINE
            } else {
                ink(c.rel)
            };
            let bg = if on { CELL_FULL } else { CELL_BG };
            g.spawn((
                Button,
                ContactButton::Pick(c.wallet.clone()),
                Node {
                    width: Val::Px(CARD_PX),
                    flex_direction: FlexDirection::Column,
                    border: UiRect::all(Val::Px(if on { 2.0 } else { 1.0 })),
                    ..default()
                },
                BackgroundColor(bg),
                Hover::on(bg),
                BorderColor::all(border),
            ))
            .with_children(|card| {
                shot(card, contacts, c, CARD_PX - 4.0);
                card.spawn((
                    Node {
                        flex_direction: FlexDirection::Column,
                        padding: UiRect::axes(Val::Px(5.0), Val::Px(3.0)),
                        ..default()
                    },
                    Pickable::IGNORE,
                ))
                .with_children(|t| {
                    let name_ink = if elo(&c.wallet) && c.rel == Rel::Seen {
                        crate::render::ui::FRIEND
                    } else {
                        ink(c.rel)
                    };
                    t.spawn((
                        Text::new(cut(&c.label(), 14)),
                        font_bold(12.0),
                        TextColor(name_ink),
                        Pickable::IGNORE,
                    ));
                    t.spawn((
                        Text::new(book::ago(now, c.last_seen)),
                        font(10.0),
                        TextColor(TEXT_DIM),
                        Pickable::IGNORE,
                    ));
                });
            });
        }
    });
}

/// A name cut to fit a card.
fn cut(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(n - 1).collect();
        t.push('.');
        t
    }
}

/// The mugshot, `px` square: the grab once it is loaded, the initial on a
/// dark tile until then (or for good, for one met off screen).
fn shot(p: &mut ChildSpawnerCommands, contacts: &Contacts, c: &Contact, px: f32) {
    let mut tile = p.spawn((
        Node {
            width: Val::Px(px),
            height: Val::Px(px),
            flex_shrink: 0.0,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        },
        BackgroundColor(Color::srgba(0.12, 0.11, 0.10, 0.95)),
        Pickable::IGNORE,
    ));
    match contacts.pic(&c.wallet) {
        Some(h) => {
            tile.insert(ImageNode::new(h));
        }
        None => {
            let initial: String = c
                .label()
                .chars()
                .find(|ch| ch.is_alphanumeric())
                .map(|ch| ch.to_uppercase().collect())
                .unwrap_or_else(|| "?".into());
            tile.with_children(|t| {
                t.spawn((
                    Text::new(initial),
                    font_bold(px * 0.42),
                    TextColor(TEXT_DIM),
                    Pickable::IGNORE,
                ));
            });
        }
    }
}

/// The picked contact, large: their mugshot, how you know them, and the
/// marks.
fn detail(
    row: &mut ChildSpawnerCommands,
    contacts: &Contacts,
    sel: Option<&Contact>,
    now: u64,
    elo: &dyn Fn(&str) -> bool,
) {
    row.spawn((
        Node {
            width: Val::Px(DETAIL_W),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(6.0),
            padding: UiRect::all(Val::Px(12.0)),
            border: UiRect::all(Val::Px(1.0)),
            align_items: AlignItems::Center,
            ..default()
        },
        BackgroundColor(super::PANEL_BG),
        BorderColor::all(LINE),
    ))
    .with_children(|d| {
        let Some(c) = sel else {
            kit::line(d, "nobody here", 13.0, TEXT_DIM);
            return;
        };
        shot(d, contacts, c, SHOT_PX);
        kit::strong(d, c.label(), 20.0, ink(c.rel));
        kit::strong(d, c.rel.label(), 12.0, ink(c.rel));
        if c.agent {
            kit::line(d, "AGENT", 11.0, LINE_HOT);
        }
        if elo(&c.wallet) {
            kit::line(
                d,
                "your friend on Elo Pros",
                11.0,
                crate::render::ui::FRIEND,
            );
        }
        kit::line(
            d,
            format!(
                "{} · last seen {}",
                book::met_line(c),
                book::ago(now, c.last_seen)
            ),
            12.0,
            TEXT,
        );
        kit::line(
            d,
            format!("first met {}", crate::friends::utc_day(c.first_seen)),
            11.0,
            TEXT_DIM,
        );
        if c.killed_me > 0 {
            let times = if c.killed_me == 1 {
                "once".to_string()
            } else {
                format!("{} times", c.killed_me)
            };
            kit::line(d, format!("killed you {times}"), 12.0, super::TEXT_SHORT);
        }
        kit::row(d, |r| {
            for rel in [Rel::Friendly, Rel::Enemy, Rel::Seen] {
                kit::button(r, rel.label(), ContactButton::Mark(rel), c.rel == rel);
            }
        });
        kit::row(d, |r| {
            kit::button(r, "VIEW PROFILE", ContactButton::Profile, true);
            kit::button(r, "FORGET", ContactButton::Forget, false);
        });
    });
}

/// Redraw the page when the book moves: a meeting, a mark, a mugshot.
pub fn watch(ui: &mut Ui, contacts: Option<&Contacts>) {
    if ui.panel != Panel::Contacts {
        return;
    }
    let gen = contacts.map_or(0, |c| c.gen);
    if gen != ui.seen.contacts_gen {
        ui.seen.contacts_gen = gen;
        ui.dirty = true;
    }
}

/// A press: a filter, a card, a page, a mark, the profile, or forgetting.
#[allow(clippy::too_many_arguments)]
pub fn clicks(
    mut ui: ResMut<Ui>,
    mut contacts: ResMut<Contacts>,
    mut card: ResMut<Profile>,
    mut friends: ResMut<Friends>,
    net: NonSend<crate::render::Net>,
    who: Res<Who>,
    menu: Res<Menu>,
    presses: Query<(&Interaction, &ContactButton), Changed<Interaction>>,
) {
    if ui.panel != Panel::Contacts {
        return;
    }
    let Some(b) = presses
        .iter()
        .find(|(i, _)| **i == Interaction::Pressed)
        .map(|(_, b)| b.clone())
    else {
        return;
    };
    let shown: Vec<String> = contacts
        .book
        .shown(ui.contacts_filter)
        .iter()
        .map(|c| c.wallet.clone())
        .collect();
    let sel = ui
        .contacts_sel
        .clone()
        .filter(|w| shown.contains(w))
        .or_else(|| shown.first().cloned());
    let sel_addr = sel.as_deref().and_then(|w| Address::from_hex(w.as_bytes()));
    match b {
        ContactButton::Filter(f) => {
            ui.contacts_filter = f;
            ui.contacts_page = 0;
        }
        ContactButton::Pick(w) => ui.contacts_sel = Some(w),
        ContactButton::Page(by) => {
            let pages = shown.len().div_ceil(PER_PAGE).max(1);
            ui.contacts_page = ui.contacts_page.saturating_add_signed(by).min(pages - 1);
        }
        ContactButton::Mark(rel) => {
            if let Some(a) = sel_addr {
                contacts.set_rel(&a, rel);
                ui.status = format!("marked {}", rel.label());
            }
        }
        ContactButton::Forget => {
            if let Some(a) = sel_addr {
                contacts.forget(&a);
                ui.contacts_sel = None;
                ui.status = "forgotten".into();
            }
        }
        ContactButton::Profile => {
            if let Some(a) = sel_addr {
                let name = contacts
                    .book
                    .get(&a)
                    .map(Contact::label)
                    .unwrap_or_default();
                let me = me_in_world(Some(&*net), &who);
                let origin = crate::ui::names::origin_of(menu.servers_url.as_deref());
                super::profile::open_card(
                    &mut ui,
                    &mut card,
                    &mut friends,
                    &origin,
                    me,
                    &net.session.core,
                    a,
                    &name,
                    Panel::Contacts,
                );
                return;
            }
        }
    }
    ui.dirty = true;
}
