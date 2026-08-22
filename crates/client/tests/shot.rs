//! Gate: the screenshot key's Bevy half actually runs, and reports honestly.
//!
//! **`crate::shot` is already gated in the code tier and none of that runs a
//! system.** The path rule and the filename rule are pure, so they are cheap
//! to assert there; what no pure test can see is whether `render/shot.rs` is
//! *wired* — whether its parameters resolve, whether the system is registered
//! at all, and whether the toast a player reads matches what reached disk.
//!
//! That gap is the one `tests/music.rs` opens with and the one `CLAUDE.md`
//! spends a whole trap on: a client fact with no call site compiles, passes
//! every workspace run, and does nothing. A screenshot key that is silently
//! not scheduled looks exactly like a screenshot key the compositor ate,
//! which is the failure this whole feature exists to rule out.
//!
//! No window, no GPU, no compositor: `MinimalPlugins` only, `tests/music.rs`'s
//! posture. **So no PNG is ever produced here, and that is the point** — with
//! no render app behind it the shot never lands, which is precisely the
//! condition the late check exists to catch. The success path is exercised by
//! writing the file this test knows the name of, from the toast the player
//! would have read.

use std::path::{Path, PathBuf};

use bevy::prelude::*;

use client::render::hud::{Rank, Toast};
use client::render::shot::{take, Shots, KEY, PENDING_CAP, VERIFY_FRAMES};

/// A directory of this test's own, under the system temp dir. Named per test
/// and per process so two tests (and two `cargo test` runs) cannot collide —
/// the whole feature is about files landing, so a shared directory would make
/// every assertion here a statement about the other test.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("gates-shot-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// The screenshot system on a bare app, aimed at `dir`.
///
/// Deliberately NOT `Shots::default()`: that reads the environment, and a test
/// that depends on this box's `HOME` is a test that passes for the wrong
/// reason on a developer's machine and fails in a container.
fn app(dir: Option<PathBuf>) -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .init_resource::<ButtonInput<KeyCode>>()
        .init_resource::<Toast>()
        .insert_resource(Shots::new(dir))
        .add_systems(Update, take);
    app
}

/// One press of the key, then one frame — with `just_pressed` cleared after,
/// because no `InputPlugin` is here to do it and a key that stays "just
/// pressed" would fire on every frame after it.
fn press(app: &mut App) {
    app.world_mut()
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KEY);
    app.update();
    let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
    keys.release(KEY);
    keys.clear();
}

/// Frames with no key down.
fn idle(app: &mut App, frames: u32) {
    for _ in 0..frames {
        app.update();
    }
}

/// The newest toast line, or `None`.
///
/// `Toast::row` is **newest first** — row 0 is the line just written, not the
/// oldest one still up (`render/hud.rs`). Reading it the other way round is
/// how the first draft of this file failed four assertions for one reason.
fn newest(app: &App) -> Option<(String, Rank)> {
    let toast = app.world().resource::<Toast>();
    toast.row(0).map(|s| (s.text().to_string(), s.rank()))
}

/// Every toast line now on screen, newest first.
fn lines(app: &App) -> Vec<(String, Rank)> {
    let toast = app.world().resource::<Toast>();
    (0..toast.len())
        .filter_map(|i| toast.row(i))
        .map(|s| (s.text().to_string(), s.rank()))
        .collect()
}

/// The file name the player was told about, from the toast the press wrote.
/// The toast is the only place the name is published, which is exactly why
/// this test reads it there — if the announced name and the written name ever
/// disagree, that is the bug worth catching.
fn announced(app: &App) -> String {
    let (text, rank) = newest(app).expect("the press said something");
    assert_eq!(rank, Rank::Note, "a shot taken is not an alarm: {text}");
    let name = text
        .rsplit_once(' ')
        .expect("the toast carries a file name")
        .1
        .to_string();
    assert!(name.starts_with("gates-"), "unexpected toast: {text}");
    assert!(name.ends_with(".png"), "unexpected toast: {text}");
    name
}

#[test]
fn the_key_makes_the_directory_and_names_a_file() {
    let dir = scratch("names");
    // The directory does NOT exist yet, deliberately: a player's first
    // screenshot is taken into a directory nothing has created, and
    // `~/Pictures` itself is absent on a fresh account.
    assert!(!dir.exists());

    let mut app = app(Some(dir.clone()));
    press(&mut app);

    assert!(dir.is_dir(), "the press did not create {}", dir.display());
    let shots = app.world().resource::<Shots>();
    assert_eq!(shots.taken, 1);
    assert_eq!(
        shots.pending(),
        1,
        "a shot is unverified until it is checked"
    );
    announced(&app);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_shot_that_never_reaches_disk_is_announced_as_a_failure() {
    // The whole reason the late check exists: `save_to_disk` handles an IO
    // error by logging and returning, so without this the client would say
    // "screenshot gates-….png" and be wrong, with nothing on screen or in the
    // exit code to say so. There is no render app here, so nothing lands.
    let dir = scratch("missing");
    let mut app = app(Some(dir.clone()));
    press(&mut app);
    let name = announced(&app);

    idle(&mut app, VERIFY_FRAMES + 1);

    let (text, rank) = newest(&app).expect("the check said something");
    assert_eq!(rank, Rank::Alarm, "a failure is not a note: {text}");
    assert!(text.contains("failed"), "unexpected toast: {text}");
    assert!(
        !dir.join(&name).exists(),
        "nothing should have written the file"
    );
    assert_eq!(
        app.world().resource::<Shots>().pending(),
        0,
        "a checked shot leaves the pending list"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_shot_that_lands_is_never_mentioned_again() {
    // The success path, with the render app's job done by hand: the check
    // must stay quiet, because the press already announced it. A second line
    // on success would train a player to ignore the one that matters.
    let dir = scratch("landed");
    let mut app = app(Some(dir.clone()));
    press(&mut app);
    let name = announced(&app);
    std::fs::write(dir.join(&name), b"not really a png, but not empty either")
        .expect("write the file the renderer would have written");

    idle(&mut app, VERIFY_FRAMES + 1);

    assert!(
        lines(&app).iter().all(|(_, r)| *r == Rank::Note),
        "a landed shot raised an alarm: {:?}",
        lines(&app)
    );
    assert_eq!(app.world().resource::<Shots>().pending(), 0);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_empty_file_is_a_failure_and_so_is_a_directory() {
    // `is_file()` AND non-empty, `capture.rs`'s rule: a directory reports a
    // non-zero length, so a size check alone would accept one, and a
    // zero-byte file is what a half-written save leaves behind.
    for (case, make) in [
        (
            "empty",
            &(|p: &Path| std::fs::write(p, b"").unwrap()) as &dyn Fn(&Path),
        ),
        (
            "dir",
            &(|p: &Path| std::fs::create_dir_all(p).unwrap()) as &dyn Fn(&Path),
        ),
    ] {
        let dir = scratch(case);
        let mut app = app(Some(dir.clone()));
        press(&mut app);
        let name = announced(&app);
        make(&dir.join(&name));

        idle(&mut app, VERIFY_FRAMES + 1);

        let (text, rank) = newest(&app).expect("the check said something");
        assert_eq!(rank, Rank::Alarm, "{case}: not an alarm: {text}");
        assert!(text.contains("failed"), "{case}: unexpected toast: {text}");

        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn the_pending_list_is_bounded_and_refuses_rather_than_overflowing() {
    // Wall 4: a cap and a stated policy, on the one path a player drives as
    // fast as they can press. The check window is `VERIFY_FRAMES` and each
    // press here costs one frame, so `PENDING_CAP + 1` presses all land
    // inside it and the last one has nowhere to go.
    let dir = scratch("cap");
    let mut app = app(Some(dir.clone()));
    assert!(
        (PENDING_CAP as u32) < VERIFY_FRAMES,
        "this test assumes the cap fills before the first check"
    );

    for _ in 0..PENDING_CAP {
        press(&mut app);
    }
    assert_eq!(app.world().resource::<Shots>().pending(), PENDING_CAP);

    press(&mut app);
    let (text, rank) = newest(&app).expect("the refusal said something");
    assert_eq!(rank, Rank::Alarm, "a refusal is not a note: {text}");
    assert!(text.contains("too many"), "unexpected toast: {text}");
    let shots = app.world().resource::<Shots>();
    assert_eq!(shots.pending(), PENDING_CAP, "the cap did not hold");
    assert_eq!(
        shots.taken, PENDING_CAP as u32,
        "a refused press is not a shot taken"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn two_shots_in_one_second_do_not_share_a_name() {
    // `shot::pick`'s suffix, through the system rather than beside it: both
    // presses are inside one second on any box, so the second must not
    // overwrite the first. The file has to exist for `pick` to see it, which
    // is what the render app would have done between them.
    let dir = scratch("twice");
    let mut app = app(Some(dir.clone()));
    press(&mut app);
    let first = announced(&app);
    std::fs::write(dir.join(&first), b"x").unwrap();
    press(&mut app);
    let second = announced(&app);

    assert_ne!(
        first, second,
        "the second shot would have overwritten the first"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn no_directory_is_a_refusal_on_screen_rather_than_a_panic() {
    // The no-`HOME` box. Persistence off for the run is `config`'s answer and
    // it is this one's too — but a player who presses the key deserves an
    // answer on screen, not only a line in a log they are not reading.
    let mut app = app(None);
    press(&mut app);

    let (text, rank) = newest(&app).expect("the press said something");
    assert_eq!(rank, Rank::Alarm, "not an alarm: {text}");
    assert!(text.contains("GATES_SHOTS_DIR"), "unexpected toast: {text}");
    assert_eq!(app.world().resource::<Shots>().taken, 0);
}

#[test]
fn a_frame_with_no_press_says_nothing() {
    // The system runs every frame on every screen; the thing it must not do
    // is anything at all until the key is pressed.
    let dir = scratch("quiet");
    let mut app = app(Some(dir.clone()));
    idle(&mut app, 30);

    assert!(lines(&app).is_empty(), "{:?}", lines(&app));
    assert_eq!(app.world().resource::<Shots>().taken, 0);
    assert!(
        !dir.exists(),
        "a run with no screenshot created {}",
        dir.display()
    );
}
