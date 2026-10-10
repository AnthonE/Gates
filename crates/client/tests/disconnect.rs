//! Gate: the shard hanging up mid-play lands the client on
//! `Screen::Disconnected`, saying why, with the world gone (`NOW.md` §0v
//! menu 2 — this used to be checked by killing a shard by hand).
//!
//! **The hangup is real, the shard is not.** `Session::replay` builds an
//! ordinary `Session` whose event lane is fed from a recording instead of a
//! socket; dropping the `Replay` drops the lane's sender, which is exactly
//! what a live session's reader task does when the connection under it dies
//! (`lib.rs`, the reader tasks; `net::drain_lane`). Everything from there on
//! is the game's own code: `input::place_eye` pumps, `Session::closed`
//! latches, and `disconnected::plugin` — the SAME registrations
//! `GatesRenderPlugin` makes — notices, tears down and builds the screen.
//!
//! No window and no GPU: `MinimalPlugins` plus states, and the resources the
//! entry chain borrows from the modules that own them.

#![cfg(all(feature = "render", feature = "native"))]

use bevy::prelude::*;
use bevy::state::app::StatesPlugin;
use client::film::{Entry, Lane, Recording, Replay};
use client::render::screen::{Connecting, Menu, Screen};
use client::render::{
    audio, bodies, clutter, decal, disconnected, fx, ghost, highlight, hud, impact, input, map,
    mobs, props, structures, terrain_mesh, verbs, viewmodel, water, world_running, Eye, Net,
    WorldEntity, WorldId,
};
use client::Session;

const ADDR: &str = "127.0.0.1:4433";
const SEED: u64 = 0x5eed_d15c;

/// How many times `OnEnter(Screen::Disconnected)` ran. Once per hangup:
/// a screen re-entered every frame would rebuild itself and re-log the loss
/// forever, and look exactly like a screen entered once.
#[derive(Resource, Default)]
struct Entered(u32);

fn count(mut n: ResMut<Entered>) {
    n.0 += 1;
}

/// A child of the world's root, which goes with it.
#[derive(Component)]
struct Hanging;

/// A session with a recording behind it, the app around it, and the
/// recording's end of the lanes — the thing to drop to hang up.
fn boot(state: Screen, entries: Vec<Entry>, with_world: bool) -> (App, Replay) {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, StatesPlugin));
    app.insert_state(state);
    // What `world_teardown` resets.
    app.init_resource::<terrain_mesh::Ring>()
        .init_resource::<props::PropRing>()
        .init_resource::<clutter::ClutterRing>()
        .init_resource::<structures::StructRing>()
        .init_resource::<ghost::Ghost>()
        .init_resource::<highlight::Highlight>()
        .init_resource::<decal::Marks>()
        .init_resource::<bodies::Bodies>()
        .init_resource::<mobs::Herd>()
        .init_resource::<Eye>()
        .init_resource::<input::Look>()
        .init_resource::<hud::Readout>()
        .init_resource::<verbs::Pad>()
        .init_resource::<verbs::HearthView>();
    // What the rest of the entry chain forgets.
    app.init_resource::<audio::Sound>()
        .init_resource::<audio::Engine>()
        .init_resource::<audio::LastHp>()
        .init_resource::<water::Sea>()
        .init_resource::<map::Island>()
        .init_resource::<viewmodel::Motion>()
        .init_resource::<viewmodel::DrawZoom>()
        .init_resource::<impact::Chips>()
        .init_resource::<fx::Fx>()
        .init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<Entered>();
    app.insert_resource(Menu::new(ADDR, None));
    app.insert_non_send_resource(Connecting {
        addr: ADDR.into(),
        ..default()
    });
    // The pump, gated as the game gates it — it reads `Net`, which the
    // teardown removes, so without the gate the frame after would panic.
    app.add_systems(Update, input::place_eye.run_if(world_running));
    disconnected::plugin(&mut app);
    app.add_systems(OnEnter(Screen::Disconnected), count);

    let (session, replay) = Session::replay(Recording {
        welcome: protocol::Welcome {
            player_id: 1,
            seed: SEED,
            tick: 0,
            dev: true,
        },
        entries,
    });
    // `menu::poll_connect`'s own two inserts.
    let world = WorldId::with_haven(SEED, *session.core.haven());
    app.insert_resource(world);
    app.insert_non_send_resource(Net {
        session,
        sel: 0,
        light: false,
    });
    if with_world {
        // A root with a child, as the world's are: the despawn is recursive.
        app.world_mut().spawn(WorldEntity).with_child(Hanging);
    }
    (app, replay)
}

fn screen(app: &App) -> Screen {
    app.world().resource::<State<Screen>>().get().clone()
}

fn count_of<C: Component>(app: &mut App) -> usize {
    let world = app.world_mut();
    world.query_filtered::<(), With<C>>().iter(world).count()
}

/// Hang up and step until the screen is up, checking the frame it takes:
/// `watch` runs after the pump, so the frame that drains the hangup is the
/// frame that asks for the screen, and the next frame enters it.
fn hang_up(app: &mut App, replay: Replay, from: Screen) {
    drop(replay);
    app.update();
    assert!(
        matches!(
            app.world().resource::<NextState<Screen>>(),
            NextState::Pending(Screen::Disconnected)
        ),
        "{from:?}: the frame that drained the hangup did not ask for the screen \
         (is `watch` still after `place_eye`?)"
    );
    app.update();
    assert_eq!(screen(app), Screen::Disconnected, "from {from:?}");
}

#[test]
fn a_shard_that_dies_mid_play_lands_on_the_disconnected_screen() {
    // Every state that holds a session: `watch` is ungated on purpose, and
    // a hangup under the Esc menu, the death screen, the map or settings
    // must land exactly as one in the world does.
    for from in [
        Screen::Loading,
        Screen::InWorld,
        Screen::Paused,
        Screen::Dead,
        Screen::Map,
        Screen::Settings,
    ] {
        let (mut app, replay) = boot(from.clone(), Vec::new(), true);
        // A live session is left alone.
        for _ in 0..3 {
            app.update();
        }
        assert_eq!(screen(&app), from, "a live session left {from:?}");
        assert_eq!(app.world().resource::<Entered>().0, 0);
        assert!(app.world().get_non_send_resource::<Net>().is_some());

        hang_up(&mut app, replay, from.clone());

        // The right reason, in the failed connect's grammar, and the menu
        // already saying the same sentence for when this screen is gone.
        let reason = app.world().resource::<disconnected::Reason>().line.clone();
        assert_eq!(
            reason,
            format!("{ADDR}: the connection was lost"),
            "{from:?}"
        );
        assert_eq!(reason, disconnected::line(ADDR));
        assert_eq!(app.world().resource::<Menu>().status, reason, "{from:?}");
        // The world is gone, and so is the dead session under it.
        assert!(
            app.world().get_non_send_resource::<Net>().is_none(),
            "{from:?}"
        );
        assert!(app.world().get_resource::<WorldId>().is_none(), "{from:?}");
        assert_eq!(count_of::<WorldEntity>(&mut app), 0, "{from:?}");
        assert_eq!(
            count_of::<Hanging>(&mut app),
            0,
            "{from:?}: the root's child stayed"
        );
        // The screen is drawn: its camera and its tree.
        let roots = count_of::<disconnected::Root>(&mut app);
        assert!(roots >= 2, "{from:?}: {roots} roots");
        assert_eq!(app.world().resource::<Entered>().0, 1);

        // And it stays entered once — no rebuild, no second teardown.
        for _ in 0..10 {
            app.update();
        }
        assert_eq!(screen(&app), Screen::Disconnected);
        assert_eq!(
            app.world().resource::<Entered>().0,
            1,
            "{from:?}: re-entered"
        );
        assert_eq!(count_of::<disconnected::Root>(&mut app), roots);
    }
}

#[test]
fn esc_goes_back_to_the_server_list_and_takes_the_screen_with_it() {
    let (mut app, replay) = boot(Screen::InWorld, Vec::new(), true);
    app.update();
    hang_up(&mut app, replay, Screen::InWorld);
    // `MinimalPlugins` has no input plugin, so the press stays "just
    // pressed" — harmless: `keys` only runs on this screen.
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::Escape);
    app.update();
    app.update();
    assert_eq!(screen(&app), Screen::Menu);
    assert_eq!(count_of::<disconnected::Root>(&mut app), 0);
    let reason = app.world().resource::<disconnected::Reason>().line.clone();
    assert_eq!(app.world().resource::<Menu>().status, reason);
}

#[test]
fn a_kick_says_the_shards_reason_even_before_the_world_was_built() {
    // The shard's last word before hanging up (wire v89) lands in the same
    // drain that reports the hangup, and the screen says it instead of a
    // bare loss. No world on purpose: a hangup before the first root spawned
    // must still remove the session, or `watch` re-enters every frame.
    let mut bytes = [0u8; 8];
    let n = protocol::encode_refuse(
        &protocol::Refuse {
            code: protocol::REFUSE_ADMIN,
        },
        &mut bytes,
    )
    .unwrap();
    let entries = vec![Entry {
        t_ms: 0.0,
        lane: Lane::Event,
        bytes: bytes[..n].to_vec(),
    }];
    let (mut app, mut replay) = boot(Screen::Loading, entries, false);
    assert_eq!(replay.feed(), 1);
    hang_up(&mut app, replay, Screen::Loading);

    let why = protocol::refuse_text(protocol::REFUSE_ADMIN).unwrap();
    let reason = app.world().resource::<disconnected::Reason>().line.clone();
    assert_eq!(reason, format!("{ADDR}: {why}"));
    assert_eq!(app.world().resource::<Menu>().status, reason);
    assert!(app.world().get_non_send_resource::<Net>().is_none());
    assert!(app.world().get_resource::<WorldId>().is_none());
    for _ in 0..10 {
        app.update();
    }
    assert_eq!(screen(&app), Screen::Disconnected);
    assert_eq!(
        app.world().resource::<Entered>().0,
        1,
        "re-entered every frame"
    );
}
