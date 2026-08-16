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

use bevy::image::Image;

use crate::ui::death::{note, rows, sentence, wake_at, woke, Death, Wake};
use crate::ui::map;

use super::hud::Toast;
use super::menu::Screen;
use super::{ui, Net, WorldId};

/// Everything this screen owns.
#[derive(Component)]
pub struct DeathRoot;

/// The row a click lands on. A component wrapper around [`Wake`], which
/// lives in `ui::death` with the table that decides which rows exist —
/// the enum cannot be a `Component` there (no Bevy in `ui`), and it must
/// not be a second enum here (two lists that can disagree about which
/// button the player pressed).
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub struct WakeRow(pub Wake);

/// How large the death screen's map is drawn, screen px.
///
/// Smaller than `Screen::Map`'s 640 for two reasons, and the second is a
/// measurement rather than taste. It answers "where are my bags" and not
/// "where is everything", so the sentence above it is what the player reads
/// first — and the whole column has to FIT: title, sentence, map, legend,
/// two rows and a footer is ~640 px of stack, against Bevy's default 720 px
/// window. At 360 the beach row sat over the hotbar in a capture; 300 buys
/// the margin back and costs nothing legible, since a 2 048 m island at
/// 300 px is ~6.8 m a pixel and a marker is 7 px wide either way.
const DEATH_MAP_PX: f32 = 300.0;

/// Bottom strip the centred column keeps clear, screen px.
///
/// **The HUD is still drawn under this screen** — the hotbar sits 18 px up
/// and is 46 px tall, and `hud.rs` spawns it as several roots rather than
/// one, so there is no single thing to hide and hiding it is a wider change
/// than this slice. `ui::screen` centres its column in the whole viewport,
/// so a tall column runs its last row straight through those cells: with
/// the map added, "click a row, or press its number" printed over the
/// hotbar in a 720 px capture. Padding the root lifts the centre instead,
/// and costs nothing on the short (bagless) shape, which does not reach
/// down that far anyway.
///
/// 82 = 18 + 46 + 18 of air. Written as the sum rather than as `82` so it
/// is obvious what moves it.
const HUD_CLEAR_PX: f32 = 18.0 + 46.0 + 18.0;

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
        toast.warn(line);
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

pub fn setup(
    mut commands: Commands,
    net: NonSend<Net>,
    world: Res<WorldId>,
    mut island: ResMut<super::map::Island>,
    mut images: ResMut<Assets<Image>>,
) {
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

    // **The list is what shapes the screen** (bag choice v0). Own-fact,
    // sent with the death that raised this screen — the deploy mirror
    // beside it knows where every bed on the island is and cannot say
    // which are ours (`protocol`'s `SUB_BAGS`).
    let bags = core.own_bags();
    let has_bag = !bags.is_empty();
    let ready = core.any_bag_ready();
    let mut marks = map::Marks::default();
    map::resolve_wake_marks(&mut marks, bags);
    // Painted once per seed and shared with `Screen::Map` — the same
    // texture, so this costs a handle clone on every death after the
    // first open of either screen.
    let texture = has_bag.then(|| island.texture(&mut images, world.seed));

    commands
        .spawn((
            DeathRoot,
            // `ui::screen`'s layout with one field moved — lifted clear of
            // the hotbar the HUD still draws under this screen.
            //
            // ⚠ **Spelt out rather than `(ui::screen(bg), Node { … })`,
            // because that shape does not compose and does not fail to
            // compile.** Bevy 0.18 PANICS at spawn on a bundle with two of
            // the same component — *"has duplicate components"*, at
            // runtime, from inside a command queue, with the system name
            // elided unless the `debug` feature is on. It was written that
            // way here first: `cargo build`, `cargo clippy` and every
            // headless gate stayed green, and the client died the moment a
            // body hit the death screen. Booting it is what found it.
            Node {
                padding: UiRect::bottom(Val::Px(HUD_CLEAR_PX)),
                ..ui::screen_node()
            },
            // Darker than the pause wash and warmer: the world is still
            // there, and you are not in it.
            BackgroundColor(Color::srgba(0.06, 0.01, 0.01, 0.88)),
        ))
        .with_children(|root| {
            root.spawn(ui::title("YOU DIED"));
            root.spawn((
                ui::strong(line, 16.0, ui::TEXT),
                Node {
                    margin: UiRect::bottom(Val::Px(14.0)),
                    ..default()
                },
            ));

            // The map, only for a player who has somewhere to wake. With
            // no bag there is nothing on it to look at and the beach is
            // not a place you get to pick, so an empty island here would
            // be decoration on the one screen nobody wants to be reading.
            if let Some(texture) = texture {
                root.spawn((
                    Node {
                        width: Val::Px(DEATH_MAP_PX),
                        height: Val::Px(DEATH_MAP_PX),
                        border: UiRect::all(Val::Px(1.0)),
                        margin: UiRect::bottom(Val::Px(10.0)),
                        ..default()
                    },
                    BorderColor::all(ui::RULE),
                    ImageNode::new(texture),
                ))
                .with_children(|frame| {
                    // **Your bags, and nothing else.** No player marker
                    // and no corpse marker: `ALPHA.md` §1's "no map
                    // position" is about where you fell, and this screen
                    // still does not say. `ui::map::resolve_wake_marks`
                    // carries the argument.
                    for m in &marks.a[..marks.count] {
                        super::map::spawn_mark(frame, m);
                    }
                });
                root.spawn((
                    ui::label(
                        if ready {
                            "solid = ready    hollow = still cooling down"
                        } else {
                            "every bag is still cooling down - this will be a beach"
                        },
                        12.0,
                        ui::FAINT,
                    ),
                    Node {
                        margin: UiRect::bottom(Val::Px(12.0)),
                        ..default()
                    },
                ));
            }

            for (i, (wake, name, detail)) in rows(has_bag).iter().enumerate() {
                root.spawn((ui::row(460.0), WakeRow(*wake)))
                    .with_children(|b| {
                        b.spawn(ui::strong(format!("{}  {}", i + 1, name), 20.0, ui::TEXT));
                        b.spawn(ui::label(*detail, 13.0, ui::DIM));
                    });
            }

            root.spawn((
                Note,
                ui::label(note(has_bag), 13.0, ui::FAINT),
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

pub fn click(
    pressed: Query<(&Interaction, &WakeRow), Changed<Interaction>>,
    mut answer: ResMut<Answer>,
) {
    if answer.sent {
        return;
    }
    for (interaction, row) in pressed.iter() {
        if *interaction == Interaction::Pressed {
            answer.chosen = Some(row.0);
        }
    }
}

pub fn keys(keyboard: Res<ButtonInput<KeyCode>>, net: NonSend<Net>, mut answer: ResMut<Answer>) {
    if answer.sent {
        return;
    }
    let has_bag = !net.session.core.own_bags().is_empty();

    // **The digits are POSITIONAL and the letters are not.** A digit means
    // the row it is drawn beside — with no bag on the island, `1` is the
    // beach, because `1` is the only row there is. The letters stay bound
    // to the anchor (the aliases the browser bound), so `F` is a bag
    // wherever the bag is drawn — and does nothing at all when no bag row
    // exists, which is what keeps an alias from pressing a button that was
    // deliberately not offered.
    //
    // **Escape is not one of them**: there is nothing to back out to, and a
    // screen you can dismiss without answering is a corpse with no exit.
    for (n, digit) in [KeyCode::Digit1, KeyCode::Digit2].into_iter().enumerate() {
        if keyboard.just_pressed(digit) {
            if let Some(wake) = wake_at(has_bag, n + 1) {
                answer.chosen = Some(wake);
            }
        }
    }
    for (key, wake) in [(KeyCode::KeyF, Wake::Bag), (KeyCode::KeyG, Wake::Beach)] {
        if keyboard.just_pressed(key) && crate::ui::death::offers(has_bag, wake) {
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
            Err(e) => toast.warn(e.to_string()),
        },
        Err(e) => toast.warn(format!("respawn would not encode ({e:?})")),
    }
}

pub fn teardown(mut commands: Commands, roots: Query<Entity, With<DeathRoot>>) {
    for e in roots.iter() {
        commands.entity(e).despawn();
    }
}

#[cfg(test)]
mod tests {
    use crate::ui::death::WAKES;

    /// `pause`'s rule: a row the keyboard cannot reach is a row half the
    /// players will not use. `keys` above binds **two digits**, so a third
    /// row in the table would ship unreachable — the digit loop is
    /// hand-written and cannot grow by itself.
    ///
    /// The reachability of each row that *is* drawn is
    /// `ui::death::every_drawn_row_is_reachable_by_its_own_digit`, where
    /// the table lives; this is the one half of it that can only be
    /// checked here, against this file's key list.
    #[test]
    fn the_digit_loop_covers_every_row_the_table_can_offer() {
        assert_eq!(
            WAKES.len(),
            2,
            "a row was added to the wake table and `keys` still binds two digits"
        );
    }
}
