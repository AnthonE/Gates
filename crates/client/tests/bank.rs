//! The sound bank, whole or spread across frames, into the engine —
//! `audio::build_bank_with` and `audio::synthesize` (audio spread v0 and
//! audio engine v0, `DECISIONS.md` §open; `NOW.md` §0web item 5).
//!
//! The desktop installs the whole bank inside `Plugin::build`; a browser tab
//! renders one cue a frame, because ~12 MB of synthesis on the one thread a
//! page has is a tab that reads as hung. Both arms are reachable natively so
//! the spread one is driven here: nothing lands at build, the beds land
//! first, every cue lands exactly once with the same samples the whole bank
//! holds, and a frame after the last cue does nothing. No `AudioPlugin`, no
//! asset, no device: what crosses is `Engine::take_installs`, which is what
//! `audio_out::flush` reads on a real boot.
//!
//! The mutants run: rendering `Cue::ALL[next]` instead of
//! `synth_order()[next]` (the samples check goes red on the first bed), and
//! skipping the `next >= CUE_COUNT` guard (the last frame goes red on an
//! index panic).
#![cfg(feature = "render")]

use bevy::prelude::*;
use client::render::audio::{build_bank_with, synth_order, synthesize, Bank, Engine, Synth, BEDS};
use client::sound::{Cue, CUE_COUNT, SAMPLE_RATE};
use client::sound_bank;

fn app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins).init_resource::<Engine>();
    app.add_systems(Update, synthesize.run_if(resource_exists::<Synth>));
    app
}

/// What crossed to the engine since the last call, in install order.
fn landed(app: &mut App) -> Vec<(Cue, Box<[i16]>)> {
    app.world_mut()
        .resource_mut::<Engine>()
        .take_installs()
        .collect()
}

#[test]
fn the_desktop_bank_is_whole_at_build() {
    let mut app = app();
    build_bank_with(&mut app, false);
    assert!(
        app.world().get_resource::<Synth>().is_none(),
        "a whole bank owes nothing"
    );
    let bank = app.world().resource::<Bank>();
    assert_eq!(bank.count(), CUE_COUNT, "the whole bank is not whole");
    let got = landed(&mut app);
    assert_eq!(got.len(), CUE_COUNT, "the engine was not handed every cue");
    let mut seen = [false; CUE_COUNT];
    for (cue, pcm) in &got {
        assert!(!seen[cue.idx()], "{cue:?} was installed twice");
        seen[cue.idx()] = true;
        let (want, takes) = sound_bank::pcm(*cue);
        assert_eq!(
            pcm.as_ref(),
            want.as_ref(),
            "{cue:?} was installed with different samples"
        );
        let bank = app.world().resource::<Bank>();
        assert!(bank.installed(*cue), "{cue:?} landed but the bank says not");
        assert_eq!(bank.takes(*cue), takes);
        assert_eq!(
            bank.len_s(*cue),
            (pcm.len() / takes as usize) as f32 / SAMPLE_RATE as f32,
            "{cue:?}: the ledger's length is not one take's"
        );
    }
    assert!(seen.iter().all(|s| *s), "a cue never reached the engine");
    // Nothing is owed after the flush, and nothing is refused.
    assert!(landed(&mut app).is_empty());
    assert_eq!(app.world().resource::<Engine>().install_refused, 0);
}

#[test]
fn the_spread_bank_fills_one_cue_a_frame_beds_first_and_then_stops() {
    let mut app = app();
    build_bank_with(&mut app, true);
    assert_eq!(app.world().resource::<Synth>().remaining(), CUE_COUNT);
    // Nothing has landed at build.
    assert_eq!(app.world().resource::<Bank>().count(), 0);
    assert!(landed(&mut app).is_empty(), "a cue was rendered at build");
    let order = synth_order();
    for (frame, cue) in order.iter().enumerate() {
        app.update();
        let got = landed(&mut app);
        assert_eq!(
            got.len(),
            1,
            "frame {frame}: {} cues crossed instead of one",
            got.len()
        );
        let (landed_cue, pcm) = &got[0];
        assert_eq!(
            landed_cue, cue,
            "frame {frame}: {landed_cue:?} landed, not {cue:?}"
        );
        assert_eq!(
            pcm.as_ref(),
            sound_bank::pcm(*cue).0.as_ref(),
            "frame {frame}: {cue:?} holds different samples"
        );
        assert_eq!(
            app.world().resource::<Synth>().remaining(),
            CUE_COUNT - frame - 1
        );
        let bank = app.world().resource::<Bank>();
        assert_eq!(bank.count(), frame + 1);
        assert!(bank.installed(*cue));
        // Nothing past this frame's cue has landed yet.
        for later in &order[frame + 1..] {
            assert!(
                !bank.installed(*later),
                "frame {frame}: {later:?} landed early"
            );
        }
    }
    // A whole bank, and a frame after the last cue changes nothing.
    assert_eq!(app.world().resource::<Bank>().count(), CUE_COUNT);
    app.update();
    assert_eq!(app.world().resource::<Synth>().remaining(), 0);
    assert!(
        landed(&mut app).is_empty(),
        "a frame past the last cue rendered"
    );
}

#[test]
fn the_order_is_the_beds_first_then_every_other_cue_once() {
    let order = synth_order();
    assert_eq!(&order[..BEDS.len()], &BEDS[..], "the beds do not lead");
    let mut seen = [false; CUE_COUNT];
    for cue in order {
        assert!(!seen[cue.idx()], "{cue:?} is rendered twice");
        seen[cue.idx()] = true;
    }
    assert!(seen.iter().all(|s| *s), "a cue is never rendered");
}
