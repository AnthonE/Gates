//! Gate: the score's Bevy half actually runs, and sends the engine a piece
//! when the director says so.
//!
//! **`sound::music` is already gated to death in `tests/sound.rs` and none of
//! that runs a system.** The director is pure, so its rules are cheap to
//! assert headless; what no pure test can see is whether `render/audio.rs`'s
//! two music systems execute at all — whether their parameters resolve,
//! whether `music_mode`'s closure is a system Bevy accepts, and whether a
//! piece the director names becomes a command to the renderer.
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
//! No window, no GPU, no socket, no audio device, no `AudioPlugin`:
//! `MinimalPlugins` and the engine's frame buffer, which is what
//! `audio_out::flush` reads on a real boot. Since audio engine v0 the level
//! a piece is held at IS visible here — it is the gain on the `Play` and on
//! every `Gain` after it — where the `AudioSink` it used to be was not.

use std::time::Duration;

use bevy::prelude::*;
use bevy::time::TimePlugin;

use client::render::audio::{build_bank, music, music_mode, Engine, Sound, MUSIC_FADE_S};
use client::render::Settings;
use client::sound::engine::{Cmd, HELD, HELD_BEDS};
use client::sound::music::{self as director, Mode};
use client::sound::{Bus, Mix};

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
    app.add_plugins(MinimalPlugins.build().disable::<TimePlugin>())
        .insert_resource(Time::<()>::default())
        .init_resource::<Sound>()
        .init_resource::<Engine>()
        .init_resource::<Settings>();
    build_bank(&mut app);
    // The bank's installs are not this file's business; take them off the
    // buffer once so `advance` reads commands alone.
    app.world_mut()
        .resource_mut::<Engine>()
        .take_installs()
        .for_each(drop);
    app.add_systems(Update, music);
    app
}

/// Advance the app by `secs`, in frames the length of a slow one — large
/// enough that a whole song is a few hundred updates rather than tens of
/// thousands, small enough that no single step crosses two section boundaries
/// — and collect every command the frames sent, in order.
fn advance(app: &mut App, secs: f32) -> Vec<Cmd> {
    let step = 0.1f32;
    let mut t = 0.0;
    let mut out = Vec::new();
    while t < secs {
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(Duration::from_secs_f32(step));
        app.update();
        out.extend(app.world_mut().resource_mut::<Engine>().take());
        t += step;
    }
    out
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

fn plays(cmds: &[Cmd]) -> Vec<(u8, client::sound::Cue, f32)> {
    cmds.iter()
        .filter_map(|c| match *c {
            Cmd::Play { slot, cue, gain } => Some((slot, cue, gain)),
            _ => None,
        })
        .collect()
}

fn stops(cmds: &[Cmd]) -> Vec<u8> {
    cmds.iter()
        .filter_map(|c| match *c {
            Cmd::Stop { slot } => Some(slot),
            _ => None,
        })
        .collect()
}

/// The whole runtime contract in one pass: the systems resolve, the world is
/// silent through the first gap, and the first song is one `Play` per
/// section, on a music slot, at the piece's table gain under the music bus.
#[test]
fn the_director_becomes_commands() {
    let mut app = app();
    // A world director opens on `FIRST_GAP_S` of silence, so this half also
    // proves the system is running *and doing nothing*, which is the state a
    // system that failed to schedule is indistinguishable from otherwise.
    let cmds = advance(&mut app, director::FIRST_GAP_S - 1.0);
    assert!(cmds.is_empty(), "music sent {cmds:?} inside the first gap");

    let cmds = advance(&mut app, 2.0);
    let first = plays(&cmds);
    assert_eq!(first.len(), 1, "the first song did not start: {cmds:?}");
    let (slot, cue, gain) = first[0];
    assert!(
        (HELD_BEDS as u8..HELD as u8).contains(&slot),
        "the piece went to slot {slot}, which is not a music slot"
    );
    assert!(cue.is_music(), "{cue:?} is not a piece");
    // Its real level, not silence: the table gain under the default mix,
    // which opens music at `MUSIC_DEFAULT` (`Settings::default`).
    let def = cue.def();
    let expect = def.gain * Mix::default().bus_gain(Bus::Music);
    assert_eq!(
        gain.to_bits(),
        expect.to_bits(),
        "the piece opened at {gain}, not its table gain under the music bus {expect}"
    );
    assert_eq!(def.bus, Bus::Music);
    // A steady piece sends nothing after its `Play`: no gain, no stop.
    assert!(
        cmds.iter().all(|c| matches!(c, Cmd::Play { .. })),
        "a steady piece sent more than its start: {cmds:?}"
    );

    // Two more sections. Pieces overlap by their tails — which is the
    // transition design, not a leak: each `Play` lands on the next slot
    // round-robin and the renderer frees a slot when its piece ends — so
    // what this asserts is that the song CONTINUES past its first section.
    let cmds = advance(&mut app, director::SECTION_S * 2.0);
    let more = plays(&cmds);
    assert!(
        more.len() >= 2,
        "the song stopped after its first section: {cmds:?}"
    );
    let mut slots: Vec<u8> = more.iter().map(|p| p.0).collect();
    slots.dedup();
    assert_eq!(slots.len(), more.len(), "consecutive pieces shared a slot");
}

/// `music_mode` is a closure returned from a function, which is the least
/// conventional thing in the audio file — this is the gate that it is a system
/// Bevy will run, and that it does what it says on both transitions.
#[test]
fn the_menu_mode_starts_immediately_and_ends_what_was_playing() {
    let mut app = app();
    // Leaving a world: the menu has no gap, so it starts a piece on top of
    // whatever was sounding, and the orphan has to fade rather than be cut.
    let cmds = advance(&mut app, director::FIRST_GAP_S + 1.0);
    let before = plays(&cmds);
    assert_eq!(before.len(), 1);
    let orphan = before[0].0;

    transition(&mut app, Mode::Menu);
    let cmds = advance(&mut app, 0.2);
    let opened = plays(&cmds);
    assert_eq!(opened.len(), 1, "the menu did not open on music: {cmds:?}");
    let menu_slot = opened[0].0;
    assert_ne!(menu_slot, orphan, "the menu's piece cut the orphan's slot");
    // The orphan is fading, not cut: gains falling on its slot, no stop yet.
    let fading: Vec<f32> = cmds
        .iter()
        .filter_map(|c| match *c {
            Cmd::Gain { slot, gain } if slot == orphan => Some(gain),
            _ => None,
        })
        .collect();
    assert!(
        fading.len() >= 2 && fading.windows(2).all(|w| w[1] < w[0]),
        "the orphan is not ramping down: {fading:?}"
    );
    assert!(stops(&cmds).is_empty(), "the orphan was cut, not faded");

    // Past the fade, the orphan is stopped and the menu's own piece is not.
    let cmds = advance(&mut app, MUSIC_FADE_S + 0.3);
    let stopped = stops(&cmds);
    assert_eq!(
        stopped,
        vec![orphan],
        "after MUSIC_FADE_S exactly the orphan is stopped, got {stopped:?}"
    );

    // And joining a world does NOT end what the menu was playing — the gap is
    // half a minute, so there is nothing about to start on top of it.
    transition(&mut app, Mode::World);
    let cmds = advance(&mut app, MUSIC_FADE_S + 0.3);
    assert!(
        stops(&cmds).is_empty(),
        "joining a world cut the menu's music instead of letting it ring out: {cmds:?}"
    );
    assert!(
        !cmds
            .iter()
            .any(|c| matches!(c, Cmd::Gain { slot, .. } if *slot == menu_slot)),
        "joining a world moved the menu piece's level: {cmds:?}"
    );
}
