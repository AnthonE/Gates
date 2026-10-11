//! Gate: the score's Bevy half actually runs, and asks the music output for
//! a track when the director says so.
//!
//! **`sound::music` is gated in `tests/sound.rs` and none of that runs a
//! system.** What no pure test can see is whether `render/audio.rs`'s two
//! music systems execute at all — whether their parameters resolve, whether
//! `music_mode`'s closure is a system Bevy accepts, and whether a track the
//! director names becomes an order on `Engine::score`, which is what
//! `audio_out::flush` and `audio_web::flush` carry out.
//!
//! No window, no GPU, no socket, no audio device: `MinimalPlugins` and the
//! engine's frame buffer.

use std::time::Duration;

use bevy::prelude::*;
use bevy::time::TimePlugin;

use client::render::audio::{music, music_mode, Engine, Score, Sound, MUSIC_FADE_S};
use client::render::Settings;
use client::sound::music::{self as director, Mode, Track};
use client::sound::{Bus, Mix};

/// The client's music systems on a bare app.
fn app() -> App {
    let mut app = App::new();
    // **`TimePlugin` disabled, and `Time` inserted by hand.** The music this
    // drives is measured in half-minutes, and a `Time` recomputed from the
    // real clock every update advances by microseconds — reaching the first
    // gap would take millions of frames. `tests/fell.rs` sidesteps the same
    // problem by never calling `update()` at all; this one has to, because
    // half of what it is testing is that the systems run *in a schedule*.
    app.add_plugins(MinimalPlugins.build().disable::<TimePlugin>())
        .insert_resource(Time::<()>::default())
        .init_resource::<Sound>()
        .init_resource::<Engine>()
        .init_resource::<Settings>();
    app.add_systems(Update, music);
    app
}

/// Advance the app by `secs` in 0.1 s frames and collect the score's order
/// after every frame.
fn advance(app: &mut App, secs: f32) -> Vec<Score> {
    let step = 0.1f32;
    let mut t = 0.0;
    let mut out = Vec::new();
    while t < secs {
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(Duration::from_secs_f32(step));
        app.update();
        out.push(app.world().resource::<Engine>().score);
        t += step;
    }
    out
}

/// Fire one of the two `OnEnter` transitions by hand.
///
/// `register_system` rather than `run_system_cached`: `music_mode` returns a
/// **capturing** closure, and Bevy's cached path is a compile-time
/// `assert!(size_of::<S>() == 0)` that a captured `Mode` fails.
fn transition(app: &mut App, mode: Mode) {
    let id = app.world_mut().register_system(music_mode(mode));
    app.world_mut().run_system(id).expect("music_mode ran");
    app.world_mut().unregister_system(id).ok();
}

fn starts(orders: &[Score]) -> Vec<Track> {
    orders.iter().filter_map(|o| o.start).collect()
}

/// The runtime contract in one pass: the systems resolve, the session opens
/// on the intro at the music bus's level, and a steady track asks for
/// nothing more.
#[test]
fn the_director_becomes_orders() {
    let mut app = app();
    let orders = advance(&mut app, 1.0);
    assert_eq!(starts(&orders), vec![director::INTRO]);
    assert!(orders.iter().all(|o| !o.stop), "{orders:?}");
    let level = Mix::default().bus_gain(Bus::Music);
    let last = orders.last().unwrap();
    assert!(
        (last.gain - level).abs() < 1e-6,
        "the intro is held at {}, not the music bus's {level}",
        last.gain
    );
    let orders = advance(&mut app, 30.0);
    assert!(starts(&orders).is_empty(), "a playing track was restarted");
}

/// Leaving a world fades the world song out and starts the intro after it;
/// joining a world from the menu leaves the intro alone.
#[test]
fn the_menu_fades_a_world_song_into_the_intro() {
    let mut app = app();
    advance(&mut app, 1.0);
    transition(&mut app, Mode::World);
    // Through the intro and the gap to the first world song.
    let mut world_song = None;
    for _ in 0..(20 * 60) {
        let orders = advance(&mut app, 1.0);
        if let Some(t) = starts(&orders).into_iter().find(|t| *t != director::INTRO) {
            world_song = Some(t);
            break;
        }
    }
    assert!(world_song.is_some(), "no world song in twenty minutes");

    transition(&mut app, Mode::Menu);
    let orders = advance(&mut app, MUSIC_FADE_S * 0.5);
    assert!(
        starts(&orders).is_empty(),
        "the intro started over the world song"
    );
    let gains: Vec<f32> = orders.iter().map(|o| o.gain).collect();
    assert!(
        gains.windows(2).all(|w| w[1] < w[0]),
        "the world song is not fading: {gains:?}"
    );
    let orders = advance(&mut app, MUSIC_FADE_S);
    let stop_at = orders.iter().position(|o| o.stop).expect("never stopped");
    assert_eq!(orders[stop_at].start, Some(director::INTRO), "{orders:?}");

    transition(&mut app, Mode::World);
    let orders = advance(&mut app, 10.0);
    assert!(
        orders.iter().all(|o| !o.stop && o.start.is_none()),
        "joining a world touched the intro: {orders:?}"
    );
}

/// A page that started the intro inside the PLAY press hands it over, and
/// the game does not start it again.
#[test]
fn an_adopted_track_is_not_restarted() {
    let mut app = app();
    app.world_mut().resource_mut::<Engine>().score.adopt = Some((director::INTRO, 40.0));
    let orders = advance(&mut app, 5.0);
    assert!(starts(&orders).is_empty(), "{orders:?}");
}
