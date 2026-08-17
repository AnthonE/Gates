//! The Bevy half of Discord rich presence: what the game is willing to say,
//! and the once-per-change handoff to the worker.
//!
//! The model — the socket, the framing, the payloads and the copy — is
//! [`crate::discord`], which is pure and not feature-gated for the reason
//! `crate::sound` is not: a thing testable only by a windowed run against a
//! running Discord is a thing with no gate. This file holds the parts that
//! genuinely need the ECS: reading [`Screen`], the body's position, the
//! shard, and the player's two settings.
//!
//! ## Bevy draws, it does not decide — and it does not announce either
//!
//! Nothing here writes gameplay state and nothing here drains a
//! single-consumer ring. The position comes off [`Eye`], which `input` has
//! already written for the camera, and the place is a pure function of it
//! (`terrain::biome`) — so this is one more reader of facts that exist, not
//! a second opinion about the world. It is deliberately **not** a `feed`
//! consumer: `render/feed.rs` exists because two systems each popped the
//! same destructive queue and the game went silent, and a presence that
//! wanted hit or toast facts would be the third reader to try.
//!
//! ## The settings are the boundary
//!
//! `discord_presence` gates everything; `discord_share_server` gates the
//! shard name and the join address, and nothing else in the client reads
//! that second flag. With it off, [`Activity::join`] is `None` and the
//! payload provably carries no address ([`crate::discord`]'s
//! `an_unshared_activity_carries_no_address`). With it on, a friend gets
//! **Ask to Join** — which is the operator's call, on the condition the
//! player makes it (2026-08-16).
//!
//! ## Not registered on a capture run
//!
//! Same rule the panels and the screenshot key follow (`render/mod.rs`): a
//! probe harness must not open a socket to whatever Discord happens to be
//! running on the box, and a visual gate's frames must not depend on it.

use bevy::prelude::*;
use sim_core::terrain;

use crate::discord::{self, Activity, Addr, Link, Place, ShardName, State};

use super::menu::{Connecting, Menu, Screen};
use super::settings::Settings;
use super::{Eye, WorldId};

/// The frame's end of the link to the presence worker. Only inserted when
/// [`discord::start`] found an application id — no resource means dark, and
/// the systems below are registered behind the same condition, so a dark
/// build costs nothing per frame rather than a branch per frame.
/// **A non-send resource, and not by preference**: [`Link`] holds a
/// `std::sync::mpsc::Receiver`, which is `Send` but `!Sync`, so it cannot be
/// a `Resource`. `menu::Connecting` is a non-send resource in this module
/// for the same class of reason, and the frame is single-threaded for both.
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
    app.insert_non_send_resource(Presence(link));
    // Every Update rather than on a state transition: `Link::send` is a
    // struct compare when nothing changed, `Activity` is `Copy` with no
    // heap in it, and running unconditionally is what makes the queue's
    // retry-next-frame overflow policy work without a second latch here.
    app.add_systems(Update, (pump, accept_invite));
    true
}

/// What this screen is, coarsely. `Settings` is the one screen that is not
/// itself an answer — it is reachable from the intro menu *and* from the Esc
/// menu, and a player changing their field of view mid-session has not left
/// the island, so read where it came from (`settings::Settings::back`)
/// rather than guessing.
///
/// Takes references because [`Screen`] is `Clone` and not `Copy`.
fn state_for(screen: &Screen, back: &Screen) -> State {
    match screen {
        Screen::Boot => State::Booting,
        Screen::Menu => State::Menu,
        Screen::Settings => match back {
            Screen::Paused => State::InWorld,
            _ => State::Menu,
        },
        Screen::Connecting | Screen::Loading => State::Joining,
        // Paused and Map are screens drawn over a session that is still
        // connected and still pumping — the player is on the island.
        Screen::InWorld | Screen::Paused | Screen::Map => State::InWorld,
        Screen::Dead => State::Dead,
        Screen::Disconnected => State::Disconnected,
    }
}

/// Where the body is standing, as a place worth naming.
///
/// `None` until the server has actually placed the body: `Eye::placed` is
/// false from the welcome until the first snapshot carrying our own entity,
/// and before that `Eye::pos` is the **world origin**, which is a real place
/// on this island — so a presence built from it would confidently name the
/// wrong biome (`Eye`'s own doc says why that flag is not an `Option`).
fn place_for(state: State, eye: &Eye, world: Option<&WorldId>) -> Option<Place> {
    if !state.in_session() || !eye.placed {
        return None;
    }
    let world = world?;
    // The CARVED ground, because this reports what the player is standing on
    // and a biome band is decided by height. An authored site's floor is cut up
    // to a few metres from the hill it replaced, which is enough to move a band
    // — so a raw read would have presence naming the terrain that used to be
    // under the pad, at exactly the place players spend their time.
    let h = terrain::ground(world.seed, &world.haven, eye.pos.x, eye.pos.z);
    let moist = terrain::moisture(world.seed, eye.pos.x, eye.pos.z);
    Some(Place::of(terrain::biome(h, moist)))
}

/// The shard's published name, when this address is one we can name.
///
/// A direct-typed address gets `None` rather than an invented label: the
/// browser's rows are the published list, and a shard the player reached by
/// typing an address is not ours to name.
fn shard_name(addr: &str, menu: &Menu) -> Option<ShardName> {
    menu.rows
        .iter()
        .find(|r| r.addr == addr)
        .map(|r| ShardName::new(&r.name))
}

fn pump(
    mut link: NonSendMut<Presence>,
    screen: Res<State_>,
    settings: Res<Settings>,
    eye: Res<Eye>,
    world: Option<Res<WorldId>>,
    menu: Res<Menu>,
    connecting: NonSend<Connecting>,
) {
    if !settings.discord_presence {
        // Off is a state, not an absence: hand over a bare `Booting` so the
        // worker overwrites whatever detail was last published rather than
        // leaving it standing.
        link.0.send(Activity::default());
        return;
    }
    let state = state_for(screen.get(), &settings.back);
    let mut activity = Activity {
        state,
        place: place_for(state, &eye, world.as_deref()),
        ..Default::default()
    };
    if settings.discord_share_server && state.in_session() {
        let addr = connecting.addr.as_str();
        if !addr.is_empty() {
            activity.join = Some(Addr::new(addr));
            activity.shard = shard_name(addr, &menu);
        }
    }
    link.0.send(activity);
}

/// A shard someone invited this player to, over Discord's **Ask to Join**.
///
/// **The address is untrusted and is treated exactly like a pasted
/// deeplink**, which is the whole security posture of this path: it arrives
/// from another process, it may have been crafted, and `shardlist::
/// check_addr` is the same whitelist `deeplink.rs` runs a stranger's link
/// through. There is no code path here an argv cannot already reach.
///
/// Only honoured from the menu. A player mid-session is not yanked out of a
/// world by someone else's click — Discord's own flow launches the game for
/// a friend who is not running it, and that path is `deeplink.rs` and
/// needs nothing here.
fn accept_invite(
    mut link: NonSendMut<Presence>,
    screen: Res<State_>,
    mut next: ResMut<NextState<Screen>>,
    mut connecting: NonSendMut<Connecting>,
) {
    let Some(addr) = link.0.poll_join() else {
        return;
    };
    if !matches!(screen.get(), Screen::Menu) {
        info!("discord: ignoring an invite while not at the menu");
        return;
    }
    let addr = addr.as_str();
    if let Err(why) = crate::shardlist::check_addr(addr) {
        warn!("discord: refusing an invite - {why}");
        return;
    }
    info!("discord: joining {addr} from an invite");
    connecting.addr = addr.to_string();
    connecting.rx = None;
    next.set(Screen::Connecting);
}

/// `State<Screen>` under a name that does not collide with
/// [`discord::State`], which this file also uses.
type State_ = bevy::prelude::State<Screen>;

#[cfg(test)]
mod tests {
    use super::*;

    /// Every screen maps to something, and the ones that are screens *over*
    /// a live session say so. This fails if `Paused` or `Map` is ever
    /// quietly re-pointed at the menu, which would tell a friend list the
    /// player had stopped playing because they opened the map.
    #[test]
    fn a_screen_over_a_live_world_still_reads_as_playing() {
        for screen in [Screen::InWorld, Screen::Paused, Screen::Map] {
            assert_eq!(
                state_for(&screen, &Screen::Menu),
                State::InWorld,
                "{screen:?} should read as in-world"
            );
        }
    }

    #[test]
    fn settings_answers_by_where_it_came_from() {
        assert_eq!(
            state_for(&Screen::Settings, &Screen::Paused),
            State::InWorld,
            "settings opened from the Esc menu is a live session"
        );
        assert_eq!(state_for(&Screen::Settings, &Screen::Menu), State::Menu);
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
                state_for(&screen, &Screen::Menu),
                State::InWorld,
                "{screen:?} should not read as in-world"
            );
        }
    }

    /// The body's place is withheld until the server has actually placed
    /// it. Before that `Eye::pos` is the world origin — a real place on this
    /// island — so a presence built from it would name the wrong biome with
    /// full confidence.
    #[test]
    fn an_unplaced_body_has_no_place() {
        let world = WorldId::new(20260731);
        let unplaced = Eye {
            placed: false,
            ..Default::default()
        };
        assert_eq!(place_for(State::InWorld, &unplaced, Some(&world)), None);
        let placed = Eye {
            placed: true,
            ..Default::default()
        };
        // Placed, in a session, with a world: now there is something to say.
        assert!(place_for(State::InWorld, &placed, Some(&world)).is_some());
        // …and never outside a session, however placed the body is.
        assert_eq!(place_for(State::Menu, &placed, Some(&world)), None);
        // …and never without a world to resolve the biome against.
        assert_eq!(place_for(State::InWorld, &placed, None), None);
    }

    /// A `Link` sends on change and stays quiet otherwise — the property the
    /// per-frame registration depends on.
    #[test]
    fn a_repeated_activity_is_not_resent() {
        // `_rx` is held: dropping the receiver would disconnect the channel
        // and `send` would take its dead-link arm instead of the one under
        // test.
        let (mut link, _rx, _jtx) = discord::test_link();
        let menu = Activity {
            state: State::Menu,
            ..Default::default()
        };
        let world = Activity {
            state: State::InWorld,
            ..Default::default()
        };
        assert!(link.send(menu));
        assert!(!link.send(menu), "a repeat is not resent");
        assert!(link.send(world));
        assert!(link.send(menu), "a return is a change");
    }

    /// Walking from the forest to the shore is a change even though the
    /// screen never moved — the detail is part of the identity, or the
    /// presence would freeze on whatever biome the player spawned in.
    #[test]
    fn moving_between_places_is_a_change() {
        let (mut link, _rx, _jtx) = discord::test_link();
        let at = |p: Place| Activity {
            state: State::InWorld,
            place: Some(p),
            ..Default::default()
        };
        assert!(link.send(at(Place::Forest)));
        assert!(!link.send(at(Place::Forest)));
        assert!(link.send(at(Place::Shore)));
    }

    /// A full queue costs a frame, never the transition. The overflow policy
    /// of `discord::QUEUE_CAP` stated as a test: the value that could not be
    /// pushed is still pending, so the next call carries it rather than the
    /// presence being stuck describing an old screen.
    #[test]
    fn a_full_queue_retries_rather_than_dropping() {
        let (mut link, rx, _jtx) = discord::test_link();
        let a = Activity {
            state: State::Menu,
            ..Default::default()
        };
        let b = Activity {
            state: State::InWorld,
            ..Default::default()
        };
        for i in 0..discord::QUEUE_CAP {
            assert!(link.send(if i % 2 == 0 { a } else { b }), "push {i} fits");
        }
        let blocked = Activity {
            state: State::Dead,
            ..Default::default()
        };
        assert!(!link.send(blocked), "the queue is full");
        let _ = rx.recv().expect("a queued activity");
        assert!(link.send(blocked), "the pending value is retried, not lost");
    }

    /// The invite path's refusal, without a window. Every address Discord
    /// could hand us goes through the same whitelist a pasted deeplink does,
    /// so the shapes `deeplink.rs` refuses are refused here too.
    #[test]
    fn an_invite_address_gets_the_deeplink_whitelist() {
        for bad in [
            "",
            "not an address",
            "https://evil.example/x",
            "host:0",
            "host:99999",
            "../../etc/passwd",
            "host:4433 --identity 0xdead",
        ] {
            assert!(
                crate::shardlist::check_addr(bad).is_err(),
                "{bad:?} should be refused"
            );
        }
        assert!(crate::shardlist::check_addr("game.moreright.xyz:4433").is_ok());
    }
}
