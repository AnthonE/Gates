//! The lore's two screens (`ARC.md` F5, F6), and the glyph text both the
//! READ screen and the island's journal draw:
//!
//! - **TALK** — `E` at a speaker: their name, what they last said, a card
//!   per offer (`ui::offer`: the skins a gift topic hands over, its price,
//!   and one button that takes it) and a button per topic of small talk.
//!   Pressing either asks the topic (`OP_TALK`); the words are the
//!   server's, composed against the island as it is now.
//! - **READ** — `E` at a stone: its text in the ancients' glyphs, each glyph
//!   you read drawn as its letter and each you do not as its shape.

use bevy::prelude::*;

use super::kit;
use super::{
    font, font_bold, Hover, Panel, Ui, CELL_BG, LINE, LINE_HOT, TEXT, TEXT_DIM, TEXT_SHORT,
};
use crate::render::feed::{Feed, Refused};
use crate::render::icons::Icons;
use crate::ui::glyphs::{self, GLYPH_COLS, GLYPH_ROWS};
use crate::ui::offer::{self, Standing};
use client_core::arc::LoreView;
use client_core::core::ClientCore;

const TALK_W: f32 = 620.0;
const READ_W: f32 = 640.0;
/// A glyph cell's edge, px.
const CELL: f32 = 3.0;
/// Characters a line of glyphs holds before it wraps.
const LINE_CHARS: usize = 30;

/// A topic button: the speaker and the topic.
#[derive(Component, Clone, Copy)]
pub struct TopicButton(pub u8, pub u8);

/// An offer card's button (`ui::offer`): the speaker and the gift topic.
/// Pressing it asks the topic, which is what takes the offer.
#[derive(Component, Clone, Copy)]
pub struct OfferButton(pub u8, pub u8);

/// Rust's green go button, the craft queue's head.
const GO: Color = Color::srgba(0.451, 0.553, 0.271, 0.92);
const GO_HOT: Color = Color::srgba(0.53, 0.64, 0.33, 0.96);
const GO_TEXT: Color = Color::srgba(0.86, 0.95, 0.70, 0.95);
/// A skin picture on an offer card, px.
const OFFER_ICON_PX: f32 = 34.0;

/// `text` in the ancients' script: a row of glyphs per line, a letter for
/// each glyph `lore` says this player reads, a shape for each it does not.
pub fn glyph_text(p: &mut ChildSpawnerCommands, text: &str, lore: &LoreView) {
    for line in glyphs::wrap(text, LINE_CHARS) {
        p.spawn(Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::FlexEnd,
            column_gap: Val::Px(3.0),
            margin: UiRect::vertical(Val::Px(3.0)),
            ..default()
        })
        .with_children(|row| {
            for ch in line.bytes() {
                glyph(row, ch, lore);
            }
        });
    }
}

/// One character: a gap, a letter or a glyph's shape.
fn glyph(row: &mut ChildSpawnerCommands, ch: u8, lore: &LoreView) {
    let w = CELL * GLYPH_COLS as f32 + 2.0;
    let h = CELL * GLYPH_ROWS as f32 + 2.0;
    if ch == b' ' {
        row.spawn(Node {
            width: Val::Px(w * 0.8),
            height: Val::Px(h),
            ..default()
        });
        return;
    }
    if lore.reads(ch) {
        row.spawn((
            Node {
                width: Val::Px(w),
                height: Val::Px(h),
                justify_content: JustifyContent::Center,
                ..default()
            },
            Pickable::IGNORE,
        ))
        .with_children(|c| {
            c.spawn((
                Text::new((ch as char).to_string()),
                super::font_bold(15.0),
                TextColor(LINE_HOT),
                Pickable::IGNORE,
            ));
        });
        return;
    }
    let g = lore.glyph_of(ch).unwrap_or(0);
    row.spawn((
        Node {
            width: Val::Px(w),
            height: Val::Px(h),
            ..default()
        },
        Pickable::IGNORE,
    ))
    .with_children(|c| {
        for r in 0..GLYPH_ROWS {
            for col in 0..GLYPH_COLS {
                if !glyphs::lit(g, col, r) {
                    continue;
                }
                c.spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(1.0 + col as f32 * CELL),
                        top: Val::Px(1.0 + r as f32 * CELL),
                        width: Val::Px(CELL),
                        height: Val::Px(CELL),
                        ..default()
                    },
                    BackgroundColor(TEXT),
                    Pickable::IGNORE,
                ));
            }
        }
    });
}

/// How much of the alphabet this player reads, in words.
pub fn reading_line(lore: &LoreView) -> String {
    let n = lore.alphabet_len as usize;
    format!(
        "you read {} of the {n} glyphs — every stone that teaches leaves you more",
        glyphs::known_count(lore.glyphs, n)
    )
}

/// `guest`: the launcher has no wallet for this player, so a gift has
/// nowhere to go (the shard says so too, and its answer counts).
pub fn build_talk(commands: &mut Commands, ui: &Ui, core: &ClientCore, icons: &Icons, guest: bool) {
    let k = ui.talk as usize;
    let Some(s) = core.lore.speakers.get(k).filter(|s| s.known) else {
        return;
    };
    let guest = guest || s.said_gift == protocol::ARC_GIFT_GUEST;
    let offers: Vec<_> = offer::offers(s, &core.skins, &core.skins_owned, guest, |item| {
        sim_core::craft::inv_count(&core.inv, item)
    })
    .collect();
    let open = offers.iter().filter(|(_, _, st)| st.open()).count();
    let right = (open > 0).then(|| match open {
        1 => "1 OFFER".to_string(),
        n => format!("{n} OFFERS"),
    });
    kit::screen(commands, TALK_W, |p| {
        kit::header(p, s.name(), right.as_deref(), &ui.status);
        if s.said_len > 0 {
            kit::section(p, s.topic(s.said_topic as usize));
            p.spawn((
                Node {
                    max_width: Val::Px(TALK_W - 30.0),
                    ..default()
                },
                Pickable::IGNORE,
            ))
            .with_children(|q| {
                kit::line(q, format!("“{}”", s.said()), 14.0, TEXT);
            });
        } else {
            kit::line(p, "…", 14.0, TEXT_DIM);
        }
        if !offers.is_empty() {
            kit::section(p, "ON OFFER");
            for (t, o, st) in &offers {
                offer_card(p, core, icons, s.topic(*t as usize), (k as u8, *t), o, *st);
            }
        }
        let plain: Vec<u8> = (0..s.n_topics)
            .filter(|&t| !s.offer(t as usize).gift)
            .collect();
        if !plain.is_empty() {
            kit::section(p, "ASK ABOUT");
            kit::row(p, |r| {
                for t in plain {
                    kit::button(
                        r,
                        s.topic(t as usize),
                        TopicButton(k as u8, t),
                        s.said_len > 0 && s.said_topic == t,
                    );
                }
            });
        }
        kit::hint(p, "[ESC] CLOSE");
    });
}

/// One offer: its title and price, the skins it hands over, what pressing
/// does, and the button, or where it stands once there is nothing to press.
fn offer_card(
    p: &mut ChildSpawnerCommands,
    core: &ClientCore,
    icons: &Icons,
    title: &str,
    (k, t): (u8, u8),
    o: &protocol::TopicOffer,
    st: Standing,
) {
    let item = |row: u16| crate::ui::craft::item_label(&core.catalog, row);
    let price = offer::price_label(o, &item(o.item));
    let (chip, chip_color) = match st {
        Standing::Owned => ("OWNED".to_string(), super::BADGE),
        Standing::Minting => ("MINTING…".to_string(), kit::BAR_FILL),
        Standing::Short { .. } => (price.clone(), TEXT_SHORT),
        Standing::Guest | Standing::Ready => (price.clone(), LINE_HOT),
    };
    p.spawn((
        Node {
            flex_direction: FlexDirection::Column,
            padding: UiRect::all(Val::Px(8.0)),
            row_gap: Val::Px(5.0),
            border: UiRect::all(Val::Px(1.0)),
            ..default()
        },
        BackgroundColor(CELL_BG),
        BorderColor::all(if st.open() { LINE_HOT } else { LINE }),
        Pickable::IGNORE,
    ))
    .with_children(|c| {
        c.spawn(Node {
            flex_direction: FlexDirection::Row,
            justify_content: JustifyContent::SpaceBetween,
            align_items: AlignItems::Center,
            ..default()
        })
        .with_children(|h| {
            h.spawn((Text::new(title), font_bold(14.0), TextColor(TEXT)));
            h.spawn((Text::new(chip), font_bold(13.0), TextColor(chip_color)));
        });
        for r in offer::rows(o, &core.skins, item) {
            let picture = icons.item(&item(r.covers));
            offer_row(c, picture, &r, st);
        }
        let note = offer::note(st, o, &price);
        let note_color = if matches!(st, Standing::Short { .. }) {
            TEXT_SHORT
        } else {
            TEXT_DIM
        };
        c.spawn((Text::new(note), font(11.0), TextColor(note_color)));
        if let Some(label) = offer::button_label(st, o, &price) {
            go_button(c, &label, OfferButton(k, t), st == Standing::Ready);
        }
    });
}

/// A skin on an offer card: its picture in its colours, its name, and
/// whether it is a look or an unlock.
fn offer_row(
    p: &mut ChildSpawnerCommands,
    picture: Option<Handle<Image>>,
    r: &offer::OfferRow,
    st: Standing,
) {
    let [red, green, blue] = crate::ui::skins::tint_factors(r.tint);
    p.spawn(Node {
        flex_direction: FlexDirection::Row,
        align_items: AlignItems::Center,
        column_gap: Val::Px(8.0),
        ..default()
    })
    .with_children(|row| {
        let mut cell = row.spawn((
            Node {
                width: Val::Px(OFFER_ICON_PX),
                height: Val::Px(OFFER_ICON_PX),
                border: UiRect::all(Val::Px(1.0)),
                ..default()
            },
            BackgroundColor(super::CELL_FULL),
            BorderColor::all(LINE),
            Pickable::IGNORE,
        ));
        cell.with_children(|c| match picture {
            Some(image) => {
                c.spawn((
                    Node {
                        width: Val::Percent(100.0),
                        height: Val::Percent(100.0),
                        ..default()
                    },
                    ImageNode {
                        image,
                        color: Color::srgb(red, green, blue),
                        ..default()
                    },
                    Pickable::IGNORE,
                ));
            }
            // No picture baked: the skin's colour as a swatch.
            None => {
                c.spawn((
                    Node {
                        margin: UiRect::all(Val::Px(7.0)),
                        flex_grow: 1.0,
                        ..default()
                    },
                    BackgroundColor(Color::srgb(red, green, blue)),
                    Pickable::IGNORE,
                ));
            }
        });
        row.spawn((
            Node {
                flex_direction: FlexDirection::Column,
                ..default()
            },
            Pickable::IGNORE,
        ))
        .with_children(|col| {
            let name_color = if st == Standing::Owned {
                TEXT_DIM
            } else {
                TEXT
            };
            col.spawn((
                Text::new(r.name.clone()),
                font_bold(13.0),
                TextColor(name_color),
            ));
            col.spawn((Text::new(r.what.clone()), font(11.0), TextColor(TEXT_DIM)));
        });
    });
}

/// The offer's button: Rust's green when one press takes it, the dim one
/// when it cannot yet (still pressable, so the speaker says why).
fn go_button<M: Component>(p: &mut ChildSpawnerCommands, label: &str, marker: M, armed: bool) {
    let (bg, hot, text) = if armed {
        (GO, GO_HOT, GO_TEXT)
    } else {
        (CELL_BG, super::CELL_HOVER, TEXT_DIM)
    };
    p.spawn((
        Button,
        marker,
        Node {
            align_self: AlignSelf::FlexStart,
            padding: UiRect::axes(Val::Px(14.0), Val::Px(5.0)),
            border: UiRect::all(Val::Px(1.0)),
            ..default()
        },
        BackgroundColor(bg),
        Hover { rest: bg, hot },
        BorderColor::all(if armed { GO_TEXT } else { LINE }),
    ))
    .with_children(|b| {
        b.spawn((
            Text::new(label),
            font_bold(13.0),
            TextColor(text),
            Pickable::IGNORE,
        ));
    });
}

pub fn build_read(commands: &mut Commands, ui: &Ui, core: &ClientCore) {
    let k = ui.read as usize;
    let lore = &*core.lore;
    let Some(ins) = lore.inscriptions.get(k).filter(|i| i.known) else {
        return;
    };
    kit::screen(commands, READ_W, |p| {
        kit::header(p, "AN INSCRIPTION", None, &ui.status);
        if ins.len == 0 {
            kit::line(p, "you trace the marks…", 13.0, TEXT_DIM);
        } else {
            glyph_text(p, ins.text(), lore);
        }
        kit::line(p, reading_line(lore), 11.0, TEXT_DIM);
        kit::hint(p, "[ESC] CLOSE");
    });
}

/// A topic press asks it, and so does an offer's button: asking a gift
/// topic is what takes it.
pub fn clicks(
    mut ui: ResMut<Ui>,
    net: NonSend<super::super::Net>,
    topics: Query<(&Interaction, &TopicButton), Changed<Interaction>>,
    offers: Query<(&Interaction, &OfferButton), Changed<Interaction>>,
) {
    if ui.panel != Panel::Talk {
        return;
    }
    let pressed = topics
        .iter()
        .map(|(i, b)| (i, b.0, b.1))
        .chain(offers.iter().map(|(i, b)| (i, b.0, b.1)));
    for (interaction, k, t) in pressed {
        if *interaction != Interaction::Pressed {
            continue;
        }
        let mut buf = [0u8; protocol::MAX_STREAM_MSG_BYTES];
        if let Ok(len) = protocol::encode_action_arc(sim_core::lore::OP_TALK, k, t, &mut buf) {
            if let Err(e) = net.session.send_action(&buf[..len]) {
                ui.say(e.to_string());
            }
        }
    }
}

/// The status line hears a refusal (walked off, say).
pub fn sync_status(feed: Res<Feed>, mut ui: ResMut<Ui>) {
    if !matches!(ui.panel, Panel::Talk | Panel::Read) {
        return;
    }
    for (which, code, _) in feed.refusals() {
        if which == Refused::Arc {
            ui.say(crate::ui::refusals::arc(code));
        }
    }
}

/// Whether the open speaker or stone is still in reach.
pub fn lore_in_reach(core: &ClientCore, panel: Panel, k: u8) -> bool {
    let [x, y, z] = core.predict.position();
    let spot = match panel {
        Panel::Talk => core.lore.speakers.get(k as usize).map(|s| s.spot),
        Panel::Read => core.lore.inscriptions.get(k as usize).map(|s| s.spot),
        _ => None,
    };
    spot.is_some_and(|s| {
        sim_core::spot::within(
            core.haven(),
            &s,
            x,
            y,
            z,
            sim_core::lore::LORE_REACH_M + 1.5,
        )
    })
}
