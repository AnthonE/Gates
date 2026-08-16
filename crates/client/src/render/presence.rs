//! The Bevy half of Discord rich presence: which screen means what, and the
//! once-per-change handoff to the worker.
//!
//! The model — the socket, the framing, the payloads and the copy — is
//! [`crate::discord`], which is pure and not feature-gated for the reason
//! `crate::sound` is not: a thing testable only by a windowed run against a
//! running Discord is a thing with no gate. This file holds the one part that
//! genuinely needs the ECS, which is reading [`Screen`].
//!
//! ## Bevy draws, it does not decide — and it does not announce either
//!
//! Nothing here reads gameplay state. The presence is a function of the
//! *screen*, which is a client-side fact about which UI is up, so this module
//! cannot become a second opinion about the world. That is deliberate and it
//! is also the privacy boundary: `discord.rs` refuses to carry an address or
//! an identity, and the way that refusal stays true is that this file never
//! has one to offer it.
//!
//! ## Not registered on a capture run
//!
//! Same rule the panels and the screenshot key follow (`render/mod.rs`): a
//! probe harness must not open a socket to whatever happens to be running on
//! the box, and a visual gate's frames must not depend on it.

use bevy::prelude::*;

use crate::discord::{self, Link};

use super::menu::Screen;
use super::settings::Settings;

/// The frame's end of the link to the presence worker. Only inserted when
/// [`discord::start`] found an application id — no resource means dark, and
/// the system below is registered behind the same condition, so a dark build
/// costs nothing per frame rather than a branch per frame.
#[derive(Resource)]
pub struct Presence(Link);

/// Start the worker and register the pump, or do nothing at all.
///
/// Returns whether presence is live, so the caller can say so once at boot
/// rather than leaving a player wondering. Dark is the shipping default and
/// is not an error — see [`crate::discord`]'s header.
pub fn register(app: &mut App) -> bool {
    let Some(link) = discord::start() else {
        return false;
    };
    app.insert_resource(Presence(link));
    // Every Update rather than on a state transition: `Link::send` is an
    // enum compare when nothing changed, and running unconditionally is what
    // makes the queue's retry-next-frame overflow policy work without a
    // second latch here to remember that a push was owed.
    app.add_systems(Update, pump);
    true
}

/// What this screen is, said plainly.
///
/// `Settings` is the one screen that is not itself an answer — it is
/// reachable from the intro menu *and* from the Esc menu, and a player
/// changing their field of view mid-session has not left the island. The
/// screen carries where it came from for exactly this reason
/// (`settings::Settings::back`), so read that rather than guessing.
///
/// Takes references because [`Screen`] is `Clone` and not `Copy` — the
/// states enum holds no scalar payload and nothing here needs to own one.
fn presence_for(screen: &Screen, back: &Screen) -> discord::Presence {
    match screen {
        Screen::Boot => discord::Presence::Booting,
        Screen::Menu => discord::Presence::Menu,
        Screen::Settings => match back {
            // Opened from the Esc menu: there is a live world behind it.
            Screen::Paused => discord::Presence::InWorld,
            _ => discord::Presence::Menu,
        },
        Screen::Connecting | Screen::Loading => discord::Presence::Joining,
        // Paused and Map are screens drawn over a session that is still
        // connected and still pumping — the player is on the island.
        Screen::InWorld | Screen::Paused | Screen::Map => discord::Presence::InWorld,
        Screen::Dead => discord::Presence::Dead,
        Screen::Disconnected => discord::Presence::Disconnected,
    }
}

fn pump(mut link: ResMut<Presence>, screen: Res<State<Screen>>, settings: Res<Settings>) {
    link.0.send(presence_for(screen.get(), &settings.back));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every screen maps to something, and the two that are screens *over* a
    /// live session say so. This is the assertion that fails if `Paused` or
    /// `Map` is ever quietly re-pointed at the menu, which would tell a
    /// friend list the player had stopped playing because they opened the
    /// map.
    #[test]
    fn a_screen_over_a_live_world_still_reads_as_playing() {
        for screen in [Screen::InWorld, Screen::Paused, Screen::Map] {
            assert_eq!(
                presence_for(&screen, &Screen::Menu),
                discord::Presence::InWorld,
                "{screen:?} should read as in-world"
            );
        }
    }

    #[test]
    fn settings_answers_by_where_it_came_from() {
        assert_eq!(
            presence_for(&Screen::Settings, &Screen::Paused),
            discord::Presence::InWorld,
            "settings opened from the Esc menu is a live session"
        );
        assert_eq!(
            presence_for(&Screen::Settings, &Screen::Menu),
            discord::Presence::Menu,
        );
    }

    #[test]
    fn the_lobby_states_are_not_in_world() {
        for screen in [
            Screen::Boot,
            Screen::Menu,
            Screen::Connecting,
            Screen::Loading,
        ] {
            assert_ne!(
                presence_for(&screen, &Screen::Menu),
                discord::Presence::InWorld,
                "{screen:?} should not read as in-world"
            );
        }
    }

    /// A `Link` sends on change and stays quiet otherwise — the property the
    /// per-frame registration depends on. Checked here rather than in
    /// `discord.rs` because it is this file's reason for running every
    /// frame.
    #[test]
    fn a_repeated_state_is_not_resent() {
        // `_rx` is held: dropping the receiver would disconnect the channel
        // and `send` would take its dead-link arm instead of the one under
        // test.
        let (mut link, _rx) = discord::test_link();
        assert!(link.send(discord::Presence::Menu));
        assert!(
            !link.send(discord::Presence::Menu),
            "a repeat is not resent"
        );
        assert!(link.send(discord::Presence::InWorld));
        assert!(link.send(discord::Presence::Menu), "a return is a change");
    }

    /// A full queue costs a frame, never the transition. This is the
    /// overflow policy of `discord::QUEUE_CAP` stated as a test: the value
    /// that could not be pushed is still pending, so the next call carries
    /// it rather than the presence being stuck describing an old screen.
    #[test]
    fn a_full_queue_retries_rather_than_dropping() {
        let (mut link, rx) = discord::test_link();
        // Fill it. Alternating so every push is a change.
        for i in 0..discord::QUEUE_CAP {
            let p = if i % 2 == 0 {
                discord::Presence::Menu
            } else {
                discord::Presence::InWorld
            };
            assert!(link.send(p), "push {i} should fit");
        }
        let blocked = discord::Presence::Dead;
        assert!(!link.send(blocked), "the queue is full");
        // Drain one slot and the same value goes through unprompted.
        let _ = rx.recv().expect("a queued presence");
        assert!(link.send(blocked), "the pending value is retried, not lost");
    }
}
