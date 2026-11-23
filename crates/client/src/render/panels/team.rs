//! The TEAM screen (`M`, wire v108): who is on your team, and the verbs the
//! shard takes for them — leave; for the leader, promote and kick; with an
//! invite waiting, join or decline. Which buttons show is `ui::team` (pure,
//! tested); a press sends the human client's own team verb, and the shard's
//! team view is the answer. With an agent on the team that takes your
//! orders, an ORDERS section types its crew orders on team chat for you
//! (`client_core::crew`). Bevy draws; nothing here decides.

use bevy::prelude::*;

use super::kit;
use super::{Panel, Ui, TEXT, TEXT_DIM};
use crate::ui::team as words;
use client_core::core::ClientCore;

/// The card's width, px.
pub const TEAM_W: f32 = 460.0;

/// A press: the verb it sends.
#[derive(Component, Clone, Copy)]
pub struct TeamButton(pub words::Button);

/// An order press: the team line it sends.
#[derive(Component, Clone)]
pub struct OrderButton(pub String);

/// The width of an ORDERS row's who cell, px.
const ORDER_WHO_W: f32 = 90.0;
/// Order buttons per line of the section: eight in one would run off the
/// card.
const ORDERS_PER_LINE: usize = 4;

pub fn build_team(commands: &mut Commands, ui: &Ui, core: &ClientCore) {
    let team = core.team();
    let me = core.player_id;
    let [x, _, z] = core.predict.render_position();
    let rows = words::rows(team, me, [x, z], |id| crate::ui::names::shown(core, id));
    kit::page(commands, ui, TEAM_W, |p| {
        let count = format!("{}/{}", team.count, protocol::TEAM_MAX);
        kit::header(
            p,
            "TEAM",
            (team.count > 0).then_some(count.as_str()),
            &ui.status,
        );
        if team.count == 0 {
            if team.invite != 0 {
                let who = crate::ui::names::shown(core, team.invite);
                kit::line(p, format!("{who} invited you to their team."), 13.0, TEXT);
            } else {
                kit::line(
                    p,
                    "You're not on a team. Look at a player and press N to invite them.",
                    12.0,
                    TEXT_DIM,
                );
            }
        }
        // The header row is the card's own header; one row per member.
        for row in rows.iter().skip(1) {
            kit::row(p, |r| {
                kit::cell(
                    r,
                    row.text.trim_start(),
                    250.0,
                    crate::render::team::ink(row.state),
                );
                for b in words::member_buttons(team, me, row.id) {
                    kit::button(r, b.label, TeamButton(b), true);
                }
            });
        }
        let orders = words::order_rows(
            team,
            me,
            |id| core.is_agent(id),
            |id| core.crew_of(id),
            |id| core.tag(id).map(|t| t.name.as_str().to_string()),
        );
        if !orders.is_empty() {
            kit::section(p, "ORDERS");
            for row in &orders {
                for (i, chunk) in row.orders.chunks(ORDERS_PER_LINE).enumerate() {
                    kit::row(p, |r| {
                        let who = if i == 0 { row.who.as_str() } else { "" };
                        kit::cell(r, who, ORDER_WHO_W, TEXT);
                        for o in chunk {
                            kit::button(r, o.label, OrderButton(o.line.clone()), true);
                        }
                    });
                }
            }
        }
        let own = words::team_buttons(team);
        if !own.is_empty() {
            kit::row(p, |r| {
                for b in own {
                    kit::button(r, b.label, TeamButton(b), true);
                }
            });
        }
        kit::hint(p, "[M] OR [ESC] CLOSE");
    });
}

/// A press sends the team verb; the shard's team view redraws the screen,
/// and a refusal comes back as a chat line. An order press sends its line
/// on team chat, and the crew answers there.
pub fn clicks(
    mut ui: ResMut<Ui>,
    net: NonSend<super::super::Net>,
    presses: Query<(&Interaction, &TeamButton), Changed<Interaction>>,
    orders: Query<(&Interaction, &OrderButton), Changed<Interaction>>,
) {
    if ui.panel != Panel::Team {
        return;
    }
    for (interaction, b) in presses.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        let b = b.0;
        let mut buf = [0u8; protocol::MAX_STREAM_MSG_BYTES];
        match protocol::encode_action_team(b.op, b.target, &mut buf) {
            Ok(len) => match net.session.send_action(&buf[..len]) {
                Ok(()) => ui.say(words::sent_line(b.op)),
                Err(e) => ui.say(e.to_string()),
            },
            Err(e) => ui.say(format!("that would not encode ({e:?})")),
        }
    }
    for (interaction, o) in orders.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        let line = o.0.as_str();
        if protocol::secret::holds_secret(line) {
            ui.say("not sent: that looks like a private key or recovery phrase");
            continue;
        }
        let mut buf = [0u8; protocol::MAX_STREAM_MSG_BYTES];
        match protocol::chat::encode_chat(line.as_bytes(), protocol::Channel::Team, &mut buf) {
            Ok(len) => match net.session.send_action(&buf[..len]) {
                Ok(()) => ui.say(words::ordered_line(line)),
                Err(e) => ui.say(e.to_string()),
            },
            Err(e) => ui.say(format!("that would not encode ({e:?})")),
        }
    }
}
