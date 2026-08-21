//! `render::report`: the key's own arithmetic — what a player may choose, and
//! that every screen has a name.
//!
//! Renderer tier because it imports `render::`, headless because none of it
//! draws. Two properties, and the first is the one that matters: **a player
//! may never file a crash.** `Kind::Crash`'s fingerprint is keyed on a panic
//! location, so a chosen one would group with real panics under an empty
//! location and quietly poison the only duplicate key that is worth anything.

use client::render::menu::Screen;
use client::render::report::{screen_slug, KINDS};
use client::report::Kind;

#[test]
fn a_player_can_never_choose_a_crash() {
    assert!(
        !KINDS.contains(&Kind::Crash),
        "the panic hook is the only author of a crash report"
    );
    assert!(!KINDS.is_empty());
}

#[test]
fn the_offered_kinds_are_distinct_and_end_on_the_honest_one() {
    let mut slugs: Vec<&str> = KINDS.iter().map(|k| k.slug()).collect();
    let n = slugs.len();
    slugs.sort_unstable();
    slugs.dedup();
    assert_eq!(slugs.len(), n, "Tab must not walk past the same kind twice");
    assert_eq!(
        KINDS[KINDS.len() - 1],
        Kind::Other,
        "the last rung is the one a player takes when none of the others fit"
    );
}

#[test]
fn every_screen_has_a_distinct_name() {
    // A bug on the death screen and a bug in the world are different bugs, so
    // the report's `place.screen` has to tell them apart. The match is
    // exhaustive, which the compiler enforces; what it cannot enforce is that
    // two arms do not answer the same word.
    let all = [
        Screen::Boot,
        Screen::Menu,
        Screen::Connecting,
        Screen::Loading,
        Screen::InWorld,
        Screen::Paused,
        Screen::Dead,
        Screen::Map,
        Screen::Settings,
        Screen::Disconnected,
    ];
    let mut names: Vec<&str> = all.iter().map(screen_slug).collect();
    let n = names.len();
    for name in &names {
        assert!(!name.is_empty());
        assert!(!name.contains(' '), "a slug is one word: {name:?}");
    }
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), n, "two screens must not report the same name");
}
