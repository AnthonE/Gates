//! The Esc menu — the intro screen, seen from inside the world.
//!
//! **It is deliberately the same object.** Same title, same row width, same
//! hover, same "press its number" contract; only the verbs differ. A player
//! who has used the server list already knows how to use this, and the two
//! screens cannot drift apart in style because they share `ui`.
//!
//! **What it fixes.** `MENUS.md` §3 has carried "Escape / options — MISSING:
//! no settings, no disconnect, no keybind list" since the audit was written,
//! and `NOW.md` §0v item 4 named the half that had become possible: the intro
//! screen is now somewhere to *return* to, so leaving the world is a state
//! change rather than an exit. Esc used to do exactly one thing in the world —
//! release the pointer — and nothing at all could take a player back to the
//! server list without killing the process.
//!
//! **Three things it is careful about.**
//!
//!   · **The input frame is zeroed on the way in.** The predictor keeps
//!     applying the last frame it was given, so pausing while holding W would
//!     otherwise walk the player through the menu — and the server, which
//!     never sees a "paused", would agree with it. The pause is a client
//!     screen, not a sim state, and this is the price of that being true.
//!   · **The world keeps pumping.** `input::place_eye` runs in this state:
//!     the session must keep reading or the connection stalls and the snapshot
//!     backlog becomes a teleport the moment the player resumes.
//!   · **The pointer comes back, and goes away again on resume.** The lock is
//!     the browser client's contract and players know it from every other
//!     game; what was missing was the second half.

use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};

use super::menu::{Connecting, Menu, Screen};
use super::settings::Settings;
use super::{ui, Net};

/// Everything this screen owns.
#[derive(Component)]
pub struct PauseRoot;

/// A verb. Ordered as drawn, and the order is the reference's: the way back
/// into the game first, the way out of it last.
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Verb {
    Resume,
    Settings,
    Disconnect,
    Quit,
}

/// The rows, with the line drawn under each one. The detail line is not
/// decoration: `Disconnect` and `Quit` are one row apart and do very
/// different things, and the intro screen's own rows established that a row
/// says what it is.
pub const VERBS: [(Verb, &str, &str); 4] = [
    (Verb::Resume, "Resume", "back to the world"),
    (
        Verb::Settings,
        "Settings",
        "view, controls, screen - and the keybind list",
    ),
    (
        Verb::Disconnect,
        "Disconnect",
        "leave the shard and go back to the server list",
    ),
    (Verb::Quit, "Quit", "close the game"),
];

/// Let go of the pointer and stop the player walking. Runs on entering the
/// state, before anything is drawn.
pub fn enter(
    mut cursor: Query<&mut CursorOptions, With<PrimaryWindow>>,
    look: Res<super::input::Look>,
    net: Option<NonSendMut<Net>>,
) {
    if let Ok(mut c) = cursor.single_mut() {
        c.grab_mode = CursorGrabMode::None;
        c.visible = true;
    }
    // The header's first bullet. Yaw and pitch are kept — they are where the
    // player is looking, not something they are doing — and the buttons and
    // both move axes go to zero. Quantized through `input`'s own two
    // functions, never a second copy: a paused frame that rounded yaw
    // differently would be a frame the server and the predictor disagree
    // about, which is the quantize-both-sides law with the stakes hidden.
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

pub fn setup(mut commands: Commands, connecting: NonSend<Connecting>) {
    let addr = if connecting.addr.is_empty() {
        String::new()
    } else {
        format!("connected to {}", connecting.addr)
    };

    // The invite. Drawn rather than copied to a clipboard because a clipboard
    // is a dependency this client does not have, and drawn *here* because
    // this is the screen a player is already on when they go to tell someone
    // where they are. `deeplink::link_for` is the one place the shareable
    // form is written, so what is on screen is what the parser accepts.
    //
    // **A link is only as good as the address it carries**, and a loopback one
    // is not shareable — a friend who follows `scry://join/gates/127.0.0.1:4433`
    // arrives at their own machine. Say nothing rather than offer a link that
    // is wrong for everyone but the person reading it.
    let invite = match connecting.addr.split_once(':').map(|(h, _)| h) {
        Some(host) if is_shareable(host) => {
            format!("invite  {}", crate::deeplink::link_for(&connecting.addr))
        }
        _ => String::new(),
    };

    // No camera: the world's `Camera3d` is up, and the UI draws against it.
    commands
        .spawn((
            PauseRoot,
            // Not opaque: the world is still there and the player has not
            // left it. A dim wash is what says "paused" rather than "loading".
            ui::screen(Color::srgba(0.02, 0.02, 0.025, 0.86)),
        ))
        .with_children(|root| {
            // The same mark the splash, the menu and the loading screen
            // carry — the Esc menu is the front screen seen from inside the
            // world, so it must not be the one place the identity changes.
            root.spawn(ui::wordmark());
            root.spawn((
                ui::label(addr, 14.0, ui::DIM),
                Node {
                    margin: UiRect::bottom(Val::Px(2.0)),
                    ..default()
                },
            ));
            root.spawn((
                ui::label(invite, 13.0, ui::FAINT),
                Node {
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
                    "click a row, or press its number    -    Esc resumes",
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

/// Mouse and keyboard, into one chosen verb.
///
/// A resource rather than two handlers acting directly, for the reason
/// `menu::Picked` exists: two input paths that each performed their own state
/// change could both fire in one frame.
#[derive(Resource, Default)]
pub struct Chosen(pub Option<Verb>);

pub fn click(rows: Query<(&Interaction, &Verb), Changed<Interaction>>, mut chosen: ResMut<Chosen>) {
    for (interaction, verb) in rows.iter() {
        if *interaction == Interaction::Pressed {
            chosen.0 = Some(*verb);
        }
    }
}

pub fn keys(keyboard: Res<ButtonInput<KeyCode>>, mut chosen: ResMut<Chosen>) {
    if keyboard.just_pressed(KeyCode::Escape) {
        chosen.0 = Some(Verb::Resume);
        return;
    }
    const DIGITS: [KeyCode; 4] = [
        KeyCode::Digit1,
        KeyCode::Digit2,
        KeyCode::Digit3,
        KeyCode::Digit4,
    ];
    for (i, k) in DIGITS.iter().enumerate() {
        if keyboard.just_pressed(*k) {
            chosen.0 = Some(VERBS[i].0);
        }
    }
}

/// Act on the verb.
pub fn act(
    mut chosen: ResMut<Chosen>,
    mut next: ResMut<NextState<Screen>>,
    mut settings: ResMut<Settings>,
    mut menu: ResMut<Menu>,
    connecting: NonSend<Connecting>,
    mut cursor: Query<&mut CursorOptions, With<PrimaryWindow>>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(verb) = chosen.0.take() else {
        return;
    };
    match verb {
        Verb::Resume => {
            // Take the pointer back, so resuming does not need a click that
            // would also swing the axe.
            if let Ok(mut c) = cursor.single_mut() {
                c.grab_mode = CursorGrabMode::Locked;
                c.visible = false;
            }
            next.set(Screen::InWorld);
        }
        Verb::Settings => {
            settings.back = Screen::Paused;
            settings.dirty = false;
            next.set(Screen::Settings);
        }
        Verb::Disconnect => {
            // The status line is the intro screen's one job: say why the
            // player is looking at it. "You left" is a reason like any other.
            menu.status = if connecting.addr.is_empty() {
                "left the world".to_string()
            } else {
                format!("left {}", connecting.addr)
            };
            next.set(Screen::Menu);
        }
        Verb::Quit => {
            exit.write(AppExit::Success);
        }
    }
}

/// Would a link to this host mean anything to someone else?
///
/// Loopback and the unspecified address resolve to *the reader's own machine*,
/// so a link carrying one is not merely useless — it is actively wrong, and it
/// fails in the most confusing way available (the friend's client connects to
/// nothing, or to their own dev shard). A private-range address is a narrower
/// case and is deliberately allowed: two people on one LAN is a real way to
/// play this, and the address is correct for exactly the audience most likely
/// to be handed it.
fn is_shareable(host: &str) -> bool {
    let h = host.trim().trim_start_matches('[').trim_end_matches(']');
    !(h.is_empty()
        || h.eq_ignore_ascii_case("localhost")
        || h == "::"
        || h == "::1"
        || h == "0.0.0.0"
        // 127.0.0.0/8, all of it — 127.0.0.1 is the common one and
        // 127.0.1.1 is what a Debian box calls itself.
        || h.strip_prefix("127.")
            .is_some_and(|rest| rest.split('.').count() == 3))
}

pub fn teardown(mut commands: Commands, roots: Query<Entity, With<PauseRoot>>) {
    for e in roots.iter() {
        commands.entity(e).despawn();
    }
}

/// Esc from the world opens this screen. Lives here rather than in `input`
/// because the verb belongs to the pause menu — `input::gather` used to own
/// Escape and did exactly one thing with it, releasing the pointer, which is
/// now this state's job to do properly.
pub fn open(keyboard: Res<ButtonInput<KeyCode>>, mut next: ResMut<NextState<Screen>>) {
    if keyboard.just_pressed(KeyCode::Escape) {
        next.set(Screen::Paused);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_drawn_row_has_a_key_that_reaches_it() {
        // The intro screen's rule, applied here: a bind the player is told
        // about must work, and a row the keyboard cannot reach is a row half
        // the players will not use. `keys` walks four digits; if a fifth verb
        // is ever drawn, this fails rather than shipping a dead row.
        assert_eq!(VERBS.len(), 4);
        // ...and no verb is drawn twice, which would make a digit ambiguous.
        for (i, a) in VERBS.iter().enumerate() {
            for b in VERBS.iter().skip(i + 1) {
                assert_ne!(a.0, b.0, "{:?} is drawn twice", a.0);
            }
        }
    }

    #[test]
    fn an_invite_is_offered_only_when_it_would_work_for_someone_else() {
        // The dev shard is the address this screen sees most often, and a
        // link to it is wrong for every reader but the one looking at it.
        for mine in [
            "127.0.0.1",
            "127.0.1.1",
            "localhost",
            "LOCALHOST",
            "0.0.0.0",
            "::1",
            "::",
            "",
        ] {
            assert!(!is_shareable(mine), "{mine:?} is not shareable");
        }
        for theirs in [
            "game.moreright.xyz",
            "10.0.0.4",
            "192.168.1.20",
            "203.0.113.7",
        ] {
            assert!(is_shareable(theirs), "{theirs:?} is shareable");
        }
        // A bracketed v6 literal arrives with its brackets on, from the same
        // `split_once(':')` that produces every other host here.
        assert!(!is_shareable("[::1]"));
        assert!(is_shareable("[2001:db8::1]"));
    }

    #[test]
    fn the_drawn_invite_is_a_link_the_parser_accepts() {
        // The screen and the parser must not be able to disagree about the
        // shareable form — this is what keeps `link_for` the single writer.
        let link = crate::deeplink::link_for("game.moreright.xyz:61234");
        assert_eq!(
            crate::deeplink::parse(&link)
                .expect("drawn link parses")
                .addr,
            "game.moreright.xyz:61234"
        );
    }

    #[test]
    fn resume_is_first_and_quit_is_last() {
        // Ordering is not cosmetic here: Esc picks `VERBS[0]` by way of
        // `Verb::Resume`, and the row nearest the pointer's rest position
        // should not be the one that closes the game.
        assert_eq!(VERBS[0].0, Verb::Resume);
        assert_eq!(VERBS[VERBS.len() - 1].0, Verb::Quit);
    }
}
