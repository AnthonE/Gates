//! Gate: the score's Bevy half actually runs, and starts a piece when the
//! director says so.
//!
//! **`sound::music` is already gated to death in `tests/sound.rs` and none of
//! that runs a system.** The director is pure, so its rules are cheap to
//! assert headless; what no pure test can see is whether `render/audio.rs`'s
//! two music systems execute at all — whether their parameters resolve,
//! whether `music_mode`'s closure is a system Bevy accepts, and whether a
//! piece the director names becomes an entity.
//!
//! That gap has cost this repo a run before, in this exact file: `build_bank`
//! documents a `Startup`-vs-`OnEnter(Loading)` ordering bug that made
//! `Res<Bank>` missing at runtime, with every gate green and the system name
//! compiled out of the message. The music system is the one audio system with
//! **no run condition at all** — it runs on the menus, where there is no
//! world, no `Net` and no listener — so it is both the most likely to be
//! scheduled somewhere surprising and the least likely to be noticed when it
//! silently does not run.
//!
//! No window, no GPU, no socket, no audio device: `MinimalPlugins` plus the
//! asset plugin, `tests/fell.rs`'s posture. Without `AudioPlugin` nothing
//! grows an `AudioSink`, so what this cannot see is the level a voice is held
//! at — `tests/sound.rs` owns the arithmetic behind that, and the mixer's own
//! `Bus::Music` gain is asserted there.

use std::time::Duration;

use bevy::asset::AssetPlugin;
use bevy::audio::AudioSource;
use bevy::prelude::*;
use bevy::time::TimePlugin;

use client::render::audio::{build_bank, music, music_mode, MusicVoice, Sound, MUSIC_FADE_S};
use client::render::Settings;
use client::sound::music::{self as director, Mode};

/// The client's music systems on a bare app.
///
/// **The bank is the expensive part and it is the point**: `build_bank` is
/// what a real boot calls, and calling it here is what makes "the resource the
/// system asks for exists when the system runs" a *tested* claim rather than
/// an argued one.
fn app() -> App {
    let mut app = App::new();
    // **`TimePlugin` disabled, and `Time` inserted by hand.** The music this
    // drives is measured in half-minutes, and a `Time` recomputed from the
    // real clock every update advances by microseconds — reaching the first
    // gap would take millions of frames. `tests/fell.rs` sidesteps the same
    // problem by never calling `update()` at all; this one has to, because
    // half of what it is testing is that the systems run *in a schedule*.
    app.add_plugins((
        MinimalPlugins.build().disable::<TimePlugin>(),
        AssetPlugin::default(),
    ))
    .insert_resource(Time::<()>::default())
    .init_asset::<AudioSource>()
    .init_resource::<Sound>()
    .init_resource::<Settings>();
    build_bank(&mut app);
    app.add_systems(Update, music);
    app
}

/// Advance the app by `secs`, in frames the length of a slow one — large
/// enough that a whole song is a few hundred updates rather than tens of
/// thousands, small enough that no single step crosses two section boundaries.
fn advance(app: &mut App, secs: f32) {
    let step = 0.1f32;
    let mut t = 0.0;
    while t < secs {
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(Duration::from_secs_f32(step));
        app.update();
        t += step;
    }
}

/// Fire one of the two `OnEnter` transitions by hand.
///
/// `register_system` rather than `run_system_cached`, and the difference is a
/// fact about the thing under test: `music_mode` returns a **capturing**
/// closure, and Bevy's cached path is a compile-time `assert!(size_of::<S>()
/// == 0)` that a captured `Mode` fails. `add_systems` has no such requirement,
/// which is why the real registration works.
fn transition(app: &mut App, mode: Mode) {
    let id = app.world_mut().register_system(music_mode(mode));
    app.world_mut().run_system(id).expect("music_mode ran");
    app.world_mut().unregister_system(id).ok();
}

fn voices(app: &mut App) -> usize {
    app.world_mut()
        .query::<&MusicVoice>()
        .iter(app.world())
        .count()
}

/// The whole runtime contract in one pass: the systems resolve, the world is
/// silent through the first gap, and the first song is one entity per section.
#[test]
fn the_director_becomes_entities() {
    let mut app = app();
    // A world director opens on `FIRST_GAP_S` of silence, so this half also
    // proves the system is running *and doing nothing*, which is the state a
    // system that failed to schedule is indistinguishable from otherwise.
    advance(&mut app, director::FIRST_GAP_S - 1.0);
    assert_eq!(voices(&mut app), 0, "music started inside the first gap");

    advance(&mut app, 2.0);
    assert_eq!(voices(&mut app), 1, "the first song did not start");

    // Two more sections. A piece is a body plus a tail and nothing despawns
    // it, so pieces overlap — which is the transition design, not a leak:
    // `PlaybackMode::Despawn` collects each one when its tail ends, and
    // without an audio device there is no sink to end, so what this can assert
    // is that the count RISES with the song rather than staying at one.
    advance(&mut app, director::SECTION_S * 2.0);
    assert!(
        voices(&mut app) >= 3,
        "the song stopped after its first section"
    );
}

/// `music_mode` is a closure returned from a function, which is the least
/// conventional thing in the audio file — this is the gate that it is a system
/// Bevy will run, and that it does what it says on both transitions.
#[test]
fn the_menu_mode_starts_immediately_and_ends_what_was_playing() {
    let mut app = app();
    // Leaving a world: the menu has no gap, so it starts a piece on top of
    // whatever was sounding, and the orphan has to fade rather than be cut.
    advance(&mut app, director::FIRST_GAP_S + 1.0);
    let before = voices(&mut app);
    assert_eq!(before, 1);

    transition(&mut app, Mode::Menu);
    advance(&mut app, 0.2);
    assert!(voices(&mut app) >= 2, "the menu did not open on music");

    // Past the fade, the orphan is gone and the menu's own piece is not.
    advance(&mut app, MUSIC_FADE_S + 0.3);
    assert!(voices(&mut app) >= 1, "the fade took the new piece too");

    // And joining a world does NOT end what the menu was playing — the gap is
    // half a minute, so there is nothing about to start on top of it.
    let held = voices(&mut app);
    transition(&mut app, Mode::World);
    advance(&mut app, MUSIC_FADE_S + 0.3);
    assert_eq!(
        voices(&mut app),
        held,
        "joining a world cut the menu's music instead of letting it ring out"
    );
}
