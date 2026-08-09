//! The involuntary disconnect — `Screen::Disconnected`.
//!
//! **What it fixes.** `NOW.md` §0v item 5: a shard that dropped the session
//! mid-play left the client sitting in a dead world. `pause::Verb::Disconnect`
//! is a verb the *player* takes; the shard taking it had no state, so the
//! world kept drawing, every verb quietly went nowhere (`SendError::Closed`
//! surfaced as a toast at best), and the only way out was knowing to press
//! Esc and guess.
//!
//! **Where the hangup is noticed.** `Session::pump` — the one place that
//! touches both receive lanes every frame — latches `Session::closed` when
//! either lane's reader task ends (`lib.rs`, `drain_lane`). [`watch`] reads
//! that latch and makes it a state change; nothing here inspects channels.
//!
//! **Why a state of its own rather than a jump to the menu.** The menu's
//! status line is one dim sentence under a title, which is the right weight
//! for "you left" — the player just chose it — and the wrong one for a world
//! that vanished out from under them. An involuntary cut lands on a screen
//! that says so and waits to be acknowledged, the reference genre's own
//! "Disconnected: reason" shape; the menu is one click past it, already
//! saying the same thing in its status line for when this screen is gone.
//!
//! **The world is torn down on entry, through the existing path.** The same
//! `world_teardown` chain `Screen::Menu` runs (that is what made this cheap —
//! `NOW.md` said so and it was right), because the session under the world is
//! dead: keeping the meshes up behind a wash, `pause`-style, would be drawing
//! a live world over a dead wire. The reason line is captured into [`Reason`]
//! *before* the transition, because teardown removes `Net` — the fact
//! outlives the thing it is about.
//!
//! **The grammar is the failed connect's**, deliberately: `poll_connect`
//! reports `"{addr}: {why}"` into the menu status, and this screen says the
//! same sentence, larger, with the two rows a player wants next. One
//! vocabulary for "the shard is not there", however you arrived at it.

use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};

use super::menu::{Connecting, Menu, Screen};
use super::{ui, Net};

/// Why the player is looking at this screen, captured at the moment the
/// hangup is noticed — `Net` (and `Connecting`'s relevance) do not survive
/// into the state, so the sentence has to.
#[derive(Resource, Default)]
pub struct Reason {
    pub line: String,
}

/// The one sentence, in `poll_connect`'s own grammar (`"{addr}: {why}"`).
///
/// The transport cannot tell a shard that shut down from a route that died —
/// both present as the reader tasks ending — so the honest `why` is the loss
/// itself, and the useful half is naming WHICH shard hung up.
pub fn line(addr: &str) -> String {
    if addr.is_empty() {
        "the connection was lost".to_string()
    } else {
        format!("{addr}: the connection was lost")
    }
}

/// Notice the dead session and make it a state change.
///
/// Runs every frame, everywhere: the guard is `Net`'s presence, not a screen
/// list, because the set of states that can hold a session (`Loading` through
/// `Map`, and `Settings` opened from the Esc menu) is exactly the set where
/// `Net` exists — `menu::poll_connect` inserts it, `world_teardown` removes
/// it, and there is no third door. A frame after this fires, teardown has
/// removed `Net` and the guard stands the system down for good.
///
/// The menu's status is written HERE, not on the way out of the screen, so
/// the sentence is the same wherever the player reads it — and so a capture
/// or scripted run that never clicks through still leaves the truth behind.
pub fn watch(
    net: Option<NonSend<Net>>,
    connecting: NonSend<Connecting>,
    mut reason: ResMut<Reason>,
    mut menu: ResMut<Menu>,
    mut next: ResMut<NextState<Screen>>,
) {
    let Some(net) = net else {
        return;
    };
    if !net.session.closed() {
        return;
    }
    reason.line = line(&connecting.addr);
    menu.status = reason.line.clone();
    warn!("gates: {} - leaving the world", reason.line);
    next.set(Screen::Disconnected);
}

/// Everything this screen owns.
#[derive(Component)]
pub struct Root;

/// A verb. The way back first, the way out last — `pause::VERBS`' rule.
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Verb {
    Back,
    Quit,
}

/// The rows, with the line drawn under each one, in the house shape the
/// intro and Esc screens established: a row says what it is.
pub const VERBS: [(Verb, &str, &str); 2] = [
    (
        Verb::Back,
        "Back to the server list",
        "pick a shard and rejoin",
    ),
    (Verb::Quit, "Quit", "close the game"),
];

/// Build the screen. Runs after `world_teardown` in the entry chain, so the
/// world — and its `Camera3d` — are already gone: this spawns its own 2D
/// camera the way the intro screen does, and the background is opaque
/// because there is genuinely nothing behind it.
pub fn setup(
    mut commands: Commands,
    reason: Res<Reason>,
    mut cursor: Query<&mut CursorOptions, With<PrimaryWindow>>,
) {
    // The pointer comes back: the world that held it locked is gone, and a
    // screen with rows on it that kept the cursor captive would be a screen
    // only the number keys could leave.
    if let Ok(mut c) = cursor.single_mut() {
        c.grab_mode = CursorGrabMode::None;
        c.visible = true;
    }

    commands.spawn((Camera2d, Root));
    commands
        .spawn((Root, ui::screen(ui::BG)))
        .with_children(|root| {
            root.spawn(ui::title("DISCONNECTED"));
            root.spawn((
                ui::label(reason.line.clone(), 15.0, ui::DIM),
                Node {
                    max_width: Val::Px(620.0),
                    margin: UiRect::bottom(Val::Px(14.0)),
                    ..default()
                },
            ));

            for (i, (verb, name, detail)) in VERBS.iter().enumerate() {
                root.spawn((ui::row(460.0), *verb)).with_children(|b| {
                    b.spawn(ui::strong(format!("{}  {}", i + 1, name), 20.0, ui::TEXT));
                    b.spawn(ui::label(*detail, 13.0, ui::DIM));
                });
            }

            root.spawn((
                ui::label(
                    "click a row, or press its number    -    Esc also goes back",
                    13.0,
                    ui::FAINT,
                ),
                Node {
                    margin: UiRect::top(Val::Px(16.0)),
                    ..default()
                },
            ));
        });
}

/// Mouse and keyboard, into one chosen verb — `pause::Chosen`'s reason: two
/// input paths that each performed their own state change could both fire in
/// one frame.
#[derive(Resource, Default)]
pub struct Chosen(pub Option<Verb>);

pub fn click(rows: Query<(&Interaction, &Verb), Changed<Interaction>>, mut chosen: ResMut<Chosen>) {
    for (interaction, verb) in rows.iter() {
        if *interaction == Interaction::Pressed {
            chosen.0 = Some(*verb);
        }
    }
}

/// Esc is `Back`, not quit: the safe read of "get me out of this screen" is
/// the one that keeps the game open — the opposite of the intro screen,
/// where Esc quits, because there the player has not lost anything.
pub fn keys(keyboard: Res<ButtonInput<KeyCode>>, mut chosen: ResMut<Chosen>) {
    if keyboard.just_pressed(KeyCode::Escape) {
        chosen.0 = Some(Verb::Back);
        return;
    }
    const DIGITS: [KeyCode; 2] = [KeyCode::Digit1, KeyCode::Digit2];
    for (i, k) in DIGITS.iter().enumerate() {
        if keyboard.just_pressed(*k) {
            chosen.0 = Some(VERBS[i].0);
        }
    }
}

/// Act on the verb. `Back` is a plain transition: the menu's own `OnEnter`
/// runs its usual chain, whose `world_teardown` finds nothing standing and
/// does nothing — the module doc says exactly this double-entry is why it
/// needs no "have we ever had a world" flag.
pub fn act(
    mut chosen: ResMut<Chosen>,
    mut next: ResMut<NextState<Screen>>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(verb) = chosen.0.take() else {
        return;
    };
    match verb {
        Verb::Back => next.set(Screen::Menu),
        Verb::Quit => {
            exit.write(AppExit::Success);
        }
    }
}

pub fn teardown(mut commands: Commands, roots: Query<Entity, With<Root>>) {
    for e in roots.iter() {
        commands.entity(e).despawn();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_drawn_row_has_a_key_that_reaches_it() {
        // The intro screen's rule: a bind the player is told about must
        // work. `keys` walks two digits; a third verb drawn without a digit
        // fails here rather than shipping a dead row.
        assert_eq!(VERBS.len(), 2);
        for (i, a) in VERBS.iter().enumerate() {
            for b in VERBS.iter().skip(i + 1) {
                assert_ne!(a.0, b.0, "{:?} is drawn twice", a.0);
            }
        }
    }

    #[test]
    fn back_is_first_and_quit_is_last() {
        // Ordering is not cosmetic: Esc means `VERBS[0]`'s verb by way of
        // `Verb::Back`, and the row nearest the pointer's rest position must
        // not be the one that closes the game — `pause`'s own rule, with the
        // stakes raised because the player did not choose this screen.
        assert_eq!(VERBS[0].0, Verb::Back);
        assert_eq!(VERBS[VERBS.len() - 1].0, Verb::Quit);
    }

    #[test]
    fn the_reason_speaks_the_failed_connects_grammar() {
        // `menu::poll_connect` reports "{addr}: {why}"; this screen must say
        // the same sentence, so the two ways of losing a shard read as one
        // vocabulary.
        let l = line("127.0.0.1:4433");
        assert!(l.starts_with("127.0.0.1:4433: "), "{l}");
        // And an addr the client never learned still yields a sentence — a
        // reason line that could be empty is a screen that can go blank.
        assert!(!line("").is_empty());
    }
}
