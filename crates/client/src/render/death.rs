//! The death screen — `Screen::Dead`.
//!
//! **Dying used to end the session.** `ClientCore::dead` was set by the
//! `Death` whose victim is this player and read by nothing, and
//! `ACT_RESPAWN` had no key: a corpse stayed a corpse, the world kept
//! drawing, and the only way back was to restart the process. It is the one
//! gap on the list that was not a missing feature but a missing *exit*.
//!
//! A `Screen` rather than a HUD overlay, and the state machine earns its keep
//! here: `input::gather` runs in `InWorld` only, so a corpse stops walking
//! and stops swinging for free — the sim refuses all of it anyway
//! (`live_slot_of`), and a client that kept sending would be predicting a
//! body the server is not moving. Meanwhile `world_running` still holds, so
//! the session keeps pumping and the world keeps streaming behind the wash.
//!
//! The sentence is [`crate::ui::death`]'s and carries no position
//! (`ALPHA.md` §1).

use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};

use crate::ui::death::{sentence, woke, Death};

use super::hud::Toast;
use super::menu::Screen;
use super::{ui, Net};

/// Everything this screen owns.
#[derive(Component)]
pub struct DeathRoot;

/// Where you wake up.
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Wake {
    Bag,
    Beach,
}

/// The rows, in the reference's order: the anchor you'd rather have first.
pub const WAKES: [(Wake, &str, &str); 2] = [
    (
        Wake::Bag,
        "Wake on your bag",
        "if one is placed and off cooldown",
    ),
    (Wake::Beach, "Wake on a beach", "the shoreline, somewhere"),
];

/// The answer in flight, and what it asked for.
///
/// **The screen latches on the press rather than on the wake**, which is the
/// browser's rule and worth keeping: a second press cannot send a second
/// action into a screen the server has already closed. The sim ignores the
/// duplicate (`world.rs`), and the player should not be left wondering
/// whether the first one took.
#[derive(Resource, Default)]
pub struct Answer {
    pub chosen: Option<Wake>,
    /// True once an answer has gone out, until the wake lands.
    pub sent: bool,
    /// What was asked for, so the wake can report which anchor answered.
    pub asked_for_bag: bool,
}

/// Raise the screen when the core says this body died. Runs in `InWorld`.
pub fn watch(net: NonSend<Net>, mut next: ResMut<NextState<Screen>>) {
    if net.session.core.dead {
        next.set(Screen::Dead);
    }
}

/// Drop the screen when the wake lands, and say which anchor answered.
///
/// Runs in `Screen::Dead`. `woke_on_bag` outlives `dead` on purpose
/// (`core.rs`): a player who asked for a bag and got a beach is told so
/// *after* the screen closes, which is the only moment the fact is worth
/// anything.
pub fn awaken(
    net: NonSend<Net>,
    answer: Res<Answer>,
    mut toast: ResMut<Toast>,
    mut next: ResMut<NextState<Screen>>,
    mut cursor: Query<&mut CursorOptions, With<PrimaryWindow>>,
) {
    if net.session.core.dead {
        return;
    }
    if let Some(line) = woke(answer.asked_for_bag, net.session.core.woke_on_bag) {
        toast.say(line);
    }
    // Take the pointer back, so waking up does not need a click that would
    // also swing the axe — `pause::act`'s rule, for the same reason.
    if let Ok(mut c) = cursor.single_mut() {
        c.grab_mode = CursorGrabMode::Locked;
        c.visible = false;
    }
    next.set(Screen::InWorld);
}

/// Let go of the pointer and stop the corpse walking.
pub fn enter(
    mut cursor: Query<&mut CursorOptions, With<PrimaryWindow>>,
    look: Res<super::input::Look>,
    mut answer: ResMut<Answer>,
    mut ui: Option<ResMut<super::panels::Ui>>,
    net: Option<NonSendMut<Net>>,
) {
    *answer = Answer::default();
    if let Ok(mut c) = cursor.single_mut() {
        c.grab_mode = CursorGrabMode::None;
        c.visible = true;
    }
    // The bag you were reading is not yours any more — it is lying on the
    // ground with your body in it. Close the panel rather than leave it
    // showing a corpse's slots behind the buttons.
    if let Some(ui) = ui.as_mut() {
        ui.panel = super::panels::Panel::None;
        ui.drag = None;
        ui.dirty = true;
    }
    // `pause::enter`'s rule: the predictor keeps applying the last frame it
    // was given, so dying mid-stride would otherwise walk the corpse through
    // the screen. Yaw and pitch are kept — where you are looking is not
    // something you are doing.
    if let Some(mut net) = net {
        let sel = net.sel;
        net.session.core.set_input(
            0,
            super::input::yaw_u16(look.yaw),
            super::input::pitch_u8(look.pitch),
            0,
            0,
            sel,
        );
    }
}

pub fn setup(mut commands: Commands, net: NonSend<Net>) {
    let core = &net.session.core;
    let line = sentence(
        &Death {
            cause: core.own_death_cause,
            killer: core.own_death_killer,
            item: core.own_death_item,
            range_cm: core.own_death_range_cm,
            own_id: core.player_id,
        },
        &core.catalog,
    );

    commands
        .spawn((
            DeathRoot,
            // Darker than the pause wash and warmer: the world is still
            // there, and you are not in it.
            ui::screen(Color::srgba(0.06, 0.01, 0.01, 0.88)),
        ))
        .with_children(|root| {
            root.spawn(ui::title("YOU DIED"));
            root.spawn((
                ui::label(line, 16.0, ui::TEXT),
                Node {
                    margin: UiRect::bottom(Val::Px(18.0)),
                    ..default()
                },
            ));

            for (i, (wake, name, detail)) in WAKES.iter().enumerate() {
                root.spawn((ui::row(460.0), *wake)).with_children(|b| {
                    b.spawn(ui::label(format!("{}  {}", i + 1, name), 20.0, ui::TEXT));
                    b.spawn(ui::label(*detail, 13.0, ui::DIM));
                });
            }

            root.spawn((
                Note,
                ui::label("click a row, or press its number", 13.0, ui::FAINT),
                Node {
                    margin: UiRect::top(Val::Px(16.0)),
                    ..default()
                },
            ));
        });
}

/// The footer line, which becomes "waking…" once an answer is out.
#[derive(Component)]
pub struct Note;

pub fn click(rows: Query<(&Interaction, &Wake), Changed<Interaction>>, mut answer: ResMut<Answer>) {
    if answer.sent {
        return;
    }
    for (interaction, wake) in rows.iter() {
        if *interaction == Interaction::Pressed {
            answer.chosen = Some(*wake);
        }
    }
}

pub fn keys(keyboard: Res<ButtonInput<KeyCode>>, mut answer: ResMut<Answer>) {
    if answer.sent {
        return;
    }
    // 1/F wakes on the bag, 2/G on a beach — the digits the other screens
    // use, plus the letter aliases the browser bound. **Escape is not one of
    // them**: there is nothing to back out to, and a screen you can dismiss
    // without answering is a corpse with no exit again.
    for (keys, wake) in [
        ([KeyCode::Digit1, KeyCode::KeyF], Wake::Bag),
        ([KeyCode::Digit2, KeyCode::KeyG], Wake::Beach),
    ] {
        if keys.iter().any(|k| keyboard.just_pressed(*k)) {
            answer.chosen = Some(wake);
        }
    }
}

/// Send the answer, once.
pub fn act(
    mut answer: ResMut<Answer>,
    net: NonSend<Net>,
    mut toast: ResMut<Toast>,
    mut notes: Query<&mut Text, With<Note>>,
) {
    let Some(wake) = answer.chosen.take() else {
        return;
    };
    if answer.sent {
        return;
    }
    let on_bag = wake == Wake::Bag;
    let mut buf = [0u8; protocol::MAX_STREAM_MSG_BYTES];
    match protocol::encode_action_respawn(on_bag, &mut buf) {
        Ok(len) => match net.session.send_action(&buf[..len]) {
            Ok(()) => {
                answer.sent = true;
                answer.asked_for_bag = on_bag;
                if let Ok(mut text) = notes.single_mut() {
                    text.0 = "waking...".to_string();
                }
            }
            // A full lane means the respawn was NOT sent (wall 4). Leaving
            // `sent` false is what lets the player press again, which is the
            // whole point of reporting rather than dropping.
            Err(e) => toast.say(e.to_string()),
        },
        Err(e) => toast.say(format!("respawn would not encode ({e:?})")),
    }
}

pub fn teardown(mut commands: Commands, roots: Query<Entity, With<DeathRoot>>) {
    for e in roots.iter() {
        commands.entity(e).despawn();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_drawn_row_has_a_key_that_reaches_it() {
        // `pause`'s rule: a row the keyboard cannot reach is a row half the
        // players will not use. `keys` binds two wakes; a third drawn here
        // fails rather than shipping a dead row.
        assert_eq!(WAKES.len(), 2);
        assert_eq!(WAKES[0].0, Wake::Bag);
        assert_eq!(WAKES[1].0, Wake::Beach);
    }
}
