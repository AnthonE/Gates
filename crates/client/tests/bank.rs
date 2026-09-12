//! The sound bank, spread across frames — `audio::build_bank_with` and
//! `audio::synthesize` (audio spread v0, `DECISIONS.md` §open; `NOW.md`
//! §0web item 8).
//!
//! The desktop builds the whole bank inside `Plugin::build`; a browser tab
//! reserves the handles there and renders one cue a frame, because 11.7 MB
//! of synthesis on the one thread a page has is a tab that reads as hung.
//! Both arms are reachable natively so the spread one is driven here: every
//! handle exists from the first frame, the beds land first, every cue lands
//! exactly once with the same bytes the whole bank would have held, and a
//! frame after the last cue does nothing. The mutants run: rendering into
//! `Cue::ALL[next]` instead of `synth_order()[next]` (the bytes check goes
//! red on the first bed), and skipping the `next >= CUE_COUNT` guard (the
//! last frame goes red on an index panic).
#![cfg(feature = "render")]

use bevy::asset::{AssetPlugin, Assets};
use bevy::audio::AudioSource;
use bevy::prelude::*;
use client::render::audio::{build_bank_with, synth_order, synthesize, Bank, Synth, BEDS};
use client::sound::{synth, Cue, CUE_COUNT};

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .init_asset::<AudioSource>();
    app.add_systems(Update, synthesize.run_if(resource_exists::<Synth>));
    app
}

fn present(app: &App, cue: Cue) -> Option<Vec<u8>> {
    let bank = app.world().resource::<Bank>();
    app.world()
        .resource::<Assets<AudioSource>>()
        .get(&bank.get(cue))
        .map(|s| s.bytes.to_vec())
}

#[test]
fn the_desktop_bank_is_whole_at_build() {
    let mut app = app();
    build_bank_with(&mut app, false);
    assert!(
        app.world().get_resource::<Synth>().is_none(),
        "a whole bank owes nothing"
    );
    for cue in Cue::ALL {
        assert!(
            present(&app, cue).is_some(),
            "{cue:?} is missing from the whole bank"
        );
    }
}

#[test]
fn the_spread_bank_fills_one_cue_a_frame_beds_first_and_then_stops() {
    let mut app = app();
    build_bank_with(&mut app, true);
    assert_eq!(app.world().resource::<Synth>().remaining(), CUE_COUNT);
    // Every handle exists from the start and no asset does.
    for cue in Cue::ALL {
        assert!(
            present(&app, cue).is_none(),
            "{cue:?} was rendered at build"
        );
    }
    let order = synth_order();
    for (frame, cue) in order.iter().enumerate() {
        app.update();
        let got =
            present(&app, *cue).unwrap_or_else(|| panic!("frame {frame}: {cue:?} did not land"));
        assert_eq!(
            got,
            synth::wav(*cue),
            "frame {frame}: {cue:?} holds different bytes"
        );
        assert_eq!(
            app.world().resource::<Synth>().remaining(),
            CUE_COUNT - frame - 1
        );
        // Nothing past this frame's cue has landed yet.
        for later in &order[frame + 1..] {
            assert!(
                present(&app, *later).is_none(),
                "frame {frame}: {later:?} landed early"
            );
        }
    }
    // A whole bank, and a frame after the last cue changes nothing.
    for cue in Cue::ALL {
        assert!(present(&app, cue).is_some());
    }
    app.update();
    assert_eq!(app.world().resource::<Synth>().remaining(), 0);
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
