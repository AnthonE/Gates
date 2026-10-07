//! The lore's two screens (`ARC.md` F5, F6), and the glyph text both the
//! READ screen and the island's journal draw:
//!
//! - **TALK** — `E` at a speaker: their name, a button per topic, and what
//!   they last said. Pressing a topic asks it (`OP_TALK`); the words are the
//!   server's, composed against the island as it is now.
//! - **READ** — `E` at a stone: its text in the ancients' glyphs, each glyph
//!   you read drawn as its letter and each you do not as its shape.

use bevy::prelude::*;

use super::kit;
use super::{Panel, Ui, LINE_HOT, TEXT, TEXT_DIM};
use crate::render::feed::{Feed, Refused};
use crate::ui::glyphs::{self, GLYPH_COLS, GLYPH_ROWS};
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

pub fn build_talk(commands: &mut Commands, ui: &Ui, core: &ClientCore) {
    let k = ui.talk as usize;
    let Some(s) = core.lore.speakers.get(k).filter(|s| s.known) else {
        return;
    };
    kit::screen(commands, TALK_W, |p| {
        kit::header(p, s.name(), None, &ui.status);
        kit::row(p, |r| {
            for t in 0..s.n_topics {
                kit::button(
                    r,
                    s.topic(t as usize),
                    TopicButton(k as u8, t),
                    s.said_len > 0 && s.said_topic == t,
                );
            }
        });
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
        kit::hint(p, "[ESC] CLOSE");
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

/// A topic press asks it.
pub fn clicks(
    mut ui: ResMut<Ui>,
    net: NonSend<super::super::Net>,
    presses: Query<(&Interaction, &TopicButton), Changed<Interaction>>,
) {
    if ui.panel != Panel::Talk {
        return;
    }
    for (interaction, b) in presses.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        let mut buf = [0u8; protocol::MAX_STREAM_MSG_BYTES];
        if let Ok(len) = protocol::encode_action_arc(sim_core::lore::OP_TALK, b.0, b.1, &mut buf) {
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
