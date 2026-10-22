//! A player's profile card (`Panel::Profile`): a click on a name in the
//! standings or the chat. Their face and name, whether they are playing,
//! their hours in Gates, where they stand on your friends list, and the
//! button that changes it.
//!
//! Words and rules are `crate::profile`; the reads are `render::profile`;
//! a friends button is `render::friends::Friends::act`, which signs in the
//! launcher. A page has no launcher to sign with, so there the button opens
//! their Elo Pros page, where the same button works.

use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use protocol::Address;

use super::kit;
use super::{Panel, Ui, BADGE, LINE_HOT, TEXT, TEXT_DIM};
use crate::friends::Op;
use crate::profile::{self as words, Rel};
use crate::render::chat::Chat;
use crate::render::friends::{hex, me_in_world, FriendPic, Friends};
use crate::render::profile::Profile;
use crate::render::screen::{Menu, Who};
use client_core::core::ClientCore;

const PROFILE_W: f32 = 440.0;
const FACE_PX: f32 = 72.0;
/// The green a player online is drawn in: the badge's.
const ONLINE: Color = BADGE;

/// A name that opens a profile card when clicked: a standings row, a chat
/// line while the composer is open.
#[derive(Component, Clone)]
pub struct ProfileOpen {
    pub wallet: Address,
    pub name: String,
}

/// A press on the card.
#[derive(Component, Clone, Copy)]
pub enum ProfileButton {
    Friend(Op),
    Page,
}

/// The platform's `scheme://host`, the friends list's rule.
fn origin(menu: &Menu) -> String {
    crate::ui::names::origin_of(menu.servers_url.as_deref())
}

/// Open the card on `wallet`, coming back to `back` on Escape. Reads their
/// face and playtime, and the player's own friends list, now: a card is
/// looked at when it opens.
#[allow(clippy::too_many_arguments)]
pub fn open_card(
    ui: &mut Ui,
    card: &mut Profile,
    friends: &mut Friends,
    origin: &str,
    me: Option<Address>,
    core: &ClientCore,
    wallet: Address,
    hint: &str,
    back: Panel,
) {
    let pic = core.tag_of_address(&wallet).map_or(0, |t| t.pic);
    card.open(origin, wallet, hint, pic);
    // The last op's answer was about somebody else.
    if !friends.acting {
        friends.status.clear();
    }
    friends.ask(origin, me);
    ui.profile_back = if back == Panel::Profile {
        Panel::None
    } else {
        back
    };
    ui.panel = Panel::Profile;
    ui.drag = None;
    ui.search_focus = false;
    ui.status.clear();
    ui.dirty = true;
}

pub fn build(
    commands: &mut Commands,
    ui: &Ui,
    core: &ClientCore,
    card: Option<&Profile>,
    friends: Option<&Friends>,
) {
    let Some((card, who)) = card.and_then(|c| Some((c, c.who?))) else {
        kit::screen(commands, PROFILE_W, |p| {
            kit::header(p, "PROFILE", None, &ui.status);
            kit::line(p, "nobody picked", 13.0, TEXT_DIM);
            kit::hint(p, "[ESC] CLOSE");
        });
        return;
    };
    let wallet = hex(who);
    let me = friends.and_then(|f| f.me);
    let rel = words::rel(me, friends.and_then(|f| f.view.as_ref()), &who);
    // Streamer mode: the card is a made-up name (the click carried it) and
    // nothing that leads back to the wallet (`ui::streamer`).
    let streamer = crate::ui::streamer::on();
    let title = if streamer {
        crate::ui::streamer::alias(&wallet.to_ascii_lowercase())
    } else {
        words::title(card.face.as_ref(), &card.hint, &wallet)
    };
    let now = crate::shot::now_secs();
    // The one line that says what just happened: a friends op's answer, the
    // page opening, or a read that failed.
    let status = friends
        .map(|f| f.status.as_str())
        .filter(|s| !s.is_empty() && !matches!(rel, Rel::SignedOut))
        .or_else(|| (!card.said.is_empty()).then_some(card.said.as_str()))
        .or(card.why.as_deref())
        .unwrap_or(&ui.status)
        .to_string();
    let here = core.tag_of_address(&who).is_some();
    let back = if ui.profile_back == Panel::None {
        "[ESC] CLOSE"
    } else {
        "[ESC] BACK"
    };

    kit::screen(commands, PROFILE_W, |p| {
        kit::header(p, "PROFILE", None, &status);
        p.spawn(Node {
            flex_direction: FlexDirection::Row,
            column_gap: Val::Px(14.0),
            align_items: AlignItems::Center,
            margin: UiRect::vertical(Val::Px(4.0)),
            ..default()
        })
        .with_children(|r| {
            face(r, who, card.pic(), &title);
            r.spawn(Node {
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(3.0),
                ..default()
            })
            .with_children(|c| {
                let ink = if matches!(rel, Rel::Friend | Rel::Buddy) {
                    crate::render::ui::FRIEND
                } else {
                    TEXT
                };
                kit::strong(c, title.clone(), 20.0, ink);
                kit::line(c, crate::friends::short_wallet(&wallet), 11.0, TEXT_DIM);
                if let Some(agent) = card.face.as_ref().and_then(words::agent_line) {
                    kit::line(c, agent, 11.0, LINE_HOT);
                }
                match card.play.as_ref() {
                    Some(play) => {
                        let seen = words::seen_line(play, now);
                        kit::strong(c, seen, 13.0, if play.online { ONLINE } else { TEXT_DIM });
                    }
                    None if card.why.is_none() => {
                        kit::line(c, "looking them up...", 12.0, TEXT_DIM)
                    }
                    None => {}
                }
            });
        });

        let rel_line = words::rel_line(rel);
        if !rel_line.is_empty() {
            let ink = match rel {
                Rel::Friend | Rel::Buddy => crate::render::ui::FRIEND,
                Rel::AskedYou => LINE_HOT,
                _ => TEXT_DIM,
            };
            kit::strong(p, rel_line, 12.0, ink);
        }

        kit::section(p, "IN GATES");
        let hours = card.play.as_ref().and_then(words::hours_line);
        kit::line(
            p,
            hours.unwrap_or_else(|| "no hours on record yet".into()),
            13.0,
            TEXT,
        );
        if here {
            kit::line(p, "has been on this island this wipe", 12.0, TEXT_DIM);
        }

        kit::row(p, |r| {
            let acting = friends.is_some_and(|f| f.acting);
            // ADD and ACCEPT are the card's point and drawn lit; the ones
            // that take something back are drawn quiet.
            for (label, op) in words::acts(rel) {
                let primary = matches!(op, Op::Ask | Op::Accept);
                kit::button(r, label, ProfileButton::Friend(*op), primary && !acting);
            }
            if !streamer {
                kit::button(r, "VIEW ON ELO PROS", ProfileButton::Page, true);
            }
        });
        kit::hint(p, back);
    });
}

/// The face tile: their picture once `Pics` has it, their initial until then.
fn face(r: &mut ChildSpawnerCommands, who: Address, pic: u32, title: &str) {
    let mut tile = r.spawn((
        Node {
            width: Val::Px(FACE_PX),
            height: Val::Px(FACE_PX),
            flex_shrink: 0.0,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            border: UiRect::all(Val::Px(1.0)),
            ..default()
        },
        BackgroundColor(super::CELL_FULL),
        BorderColor::all(super::LINE),
    ));
    if pic != 0 {
        tile.insert(FriendPic { address: who, pic });
    }
    let initial: String = title
        .chars()
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_uppercase().collect())
        .unwrap_or_else(|| "?".into());
    tile.with_children(|t| {
        t.spawn((
            Text::new(initial),
            super::font_bold(FACE_PX * 0.5),
            TextColor(TEXT_DIM),
            Pickable::IGNORE,
        ));
    });
}

/// A click on a name (a standings row) opens its card.
#[allow(clippy::too_many_arguments)]
pub fn opens(
    mut ui: ResMut<Ui>,
    mut card: ResMut<Profile>,
    mut friends: ResMut<Friends>,
    net: NonSend<crate::render::Net>,
    who: Res<Who>,
    menu: Res<Menu>,
    presses: Query<(&Interaction, &ProfileOpen), Changed<Interaction>>,
) {
    let Some(p) = presses
        .iter()
        .find(|(i, _)| **i == Interaction::Pressed)
        .map(|(_, p)| p.clone())
    else {
        return;
    };
    let me = me_in_world(Some(&*net), &who);
    let back = ui.panel;
    open_card(
        &mut ui,
        &mut card,
        &mut friends,
        &origin(&menu),
        me,
        &net.session.core,
        p.wallet,
        &p.name,
        back,
    );
}

/// A click on a chat line while the composer is open: the composer shuts and
/// the speaker's card opens. The pointer stays free; the card owns it now.
#[allow(clippy::too_many_arguments)]
pub fn chat_opens(
    mut ui: ResMut<Ui>,
    mut card: ResMut<Profile>,
    mut friends: ResMut<Friends>,
    mut chat: ResMut<Chat>,
    net: NonSend<crate::render::Net>,
    who: Res<Who>,
    menu: Res<Menu>,
    presses: Query<(&Interaction, &crate::render::chat::ChatWho), Changed<Interaction>>,
) {
    if !chat.open() {
        return;
    }
    let Some(p) = presses
        .iter()
        .find(|(i, _)| **i == Interaction::Pressed)
        .map(|(_, p)| p.0.clone())
    else {
        return;
    };
    // Shut without `chat::close`'s re-lock: the card owns the pointer now,
    // and `relock` gives it back when the card is gone.
    chat.composer.open = false;
    chat.composer.clear();
    chat.dirty = true;
    let me = me_in_world(Some(&*net), &who);
    open_card(
        &mut ui,
        &mut card,
        &mut friends,
        &origin(&menu),
        me,
        &net.session.core,
        p.wallet,
        &p.name,
        Panel::None,
    );
    ui.profile_relock = true;
}

/// The pointer, back to the player once a card opened from chat is gone and
/// no panel is up. The composer released it before the card opened, so the
/// panels' own give-back (`ui::pointer`) remembers it as never captured.
pub fn relock(mut ui: ResMut<Ui>, mut cursor: Query<&mut CursorOptions, With<PrimaryWindow>>) {
    if !ui.profile_relock || ui.panel != Panel::None {
        return;
    }
    ui.profile_relock = false;
    if let Ok(mut c) = cursor.single_mut() {
        c.grab_mode = CursorGrabMode::Locked;
        c.visible = crate::render::web::in_post();
    }
}

/// A press on the card: a friends op, or their page.
pub fn clicks(
    mut ui: ResMut<Ui>,
    mut card: ResMut<Profile>,
    mut friends: ResMut<Friends>,
    menu: Res<Menu>,
    presses: Query<(&Interaction, &ProfileButton), Changed<Interaction>>,
) {
    if ui.panel != Panel::Profile {
        return;
    }
    let Some(who) = card.who else {
        return;
    };
    for (interaction, b) in presses.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        match *b {
            ProfileButton::Page => card.open_page(&origin(&menu)),
            ProfileButton::Friend(op) => friend_op(&mut card, &mut friends, &menu, op, who),
        }
        ui.dirty = true;
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn friend_op(_card: &mut Profile, friends: &mut Friends, _menu: &Menu, op: Op, who: Address) {
    friends.act(op, &hex(who));
}

/// A page signs with the wallet in the browser, on Elo Pros: the button
/// opens their page there, which has the same one.
#[cfg(target_arch = "wasm32")]
fn friend_op(card: &mut Profile, _friends: &mut Friends, menu: &Menu, _op: Op, _who: Address) {
    card.open_page(&origin(menu));
}

/// Redraw the card when a read lands or the friends list moves.
pub fn watch(ui: &mut Ui, card: Option<&Profile>, friends: Option<&Friends>) {
    if ui.panel != Panel::Profile {
        return;
    }
    let now = (card.map_or(0, |c| c.gen), friends.map_or(0, |f| f.gen));
    if now != ui.seen.profile_gen {
        ui.seen.profile_gen = now;
        ui.dirty = true;
    }
}
