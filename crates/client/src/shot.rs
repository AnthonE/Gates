//! Where a player's screenshot goes and what it is called.
//!
//! **Why the game takes its own screenshot at all.** On Linux the desktop's
//! screenshot key is not one thing: X11 tools (`scrot`, `import`, `xwd`) read
//! the root window and are simply blind under Wayland, which forbids a client
//! reading another client's pixels; the Wayland answer is a portal
//! (`xdg-desktop-portal` + the compositor's own grabber), which a bare WM,
//! a kiosk session or a container may not have. A fullscreen game makes it
//! worse — some compositors hand back a black frame for a surface on a
//! direct-scanout path, which looks exactly like a working key that captures
//! nothing. **None of that is a property of this game, but all of it is the
//! player's problem**, and the fix that survives every branch of it is for the
//! client to read its own swapchain, which is a thing it already does six
//! times per capture run (`render/capture.rs`). This module is the half of
//! that which is arithmetic.
//!
//! **NOT feature-gated**, for `config`'s reason exactly: a path rule and a
//! filename rule are pure, they are the part that can be wrong on a box
//! nobody here owns, and a test behind `--features render` runs in the
//! renderer tier where nobody looks at it. `render/shot.rs` is the Bevy half
//! — one keypress, one `Screenshot` entity.
//!
//! **The stamp is UTC and says so in no way the player can see**, which is a
//! deliberate trade rather than an oversight: a local-time name needs a
//! timezone database this client does not link, and the failure mode of
//! guessing is a filename that is wrong by hours twice a year. What the name
//! has to be is *unique* and *sortable*, and UTC is both.

use std::path::{Path, PathBuf};

use crate::config::{HostOs, HOST_OS};

/// Every screenshot starts with this, so a directory a player also drops
/// other images in still sorts and globs cleanly.
pub const PREFIX: &str = "gates-";

/// How many shots one second may hold before [`pick`] gives up.
///
/// Wall 4 wants a cap and a stated overflow policy on anything a player can
/// drive, and the screenshot key is held down as readily as any other. The
/// policy is **refuse and say so**: the alternative — walking until a name is
/// free — is an unbounded `stat` loop on the frame a key was pressed, and a
/// player who has taken 64 shots inside one second is leaning on the key
/// rather than photographing 64 things.
pub const PER_STAMP: u32 = 64;

/// Where this box's screenshots go, or `None` when the environment names no
/// base to put them in.
///
/// `None` disables the key for the run rather than writing beside the binary:
/// the depot directory is the launcher's to replace wholesale on update
/// (`config`'s header), so a screenshot saved there is a screenshot the next
/// patch deletes without telling anyone.
pub fn dir() -> Option<PathBuf> {
    dir_from(HOST_OS, |k| std::env::var(k).ok())
}

/// The resolution rule, with the environment injected so all three branches
/// are testable on one box — `config::path_from`'s shape, and for its reason:
/// a path rule only its own OS can check is a path rule CI never checks.
///
/// An empty variable counts as unset (the XDG rule, applied to all of them),
/// because an empty `HOME` would otherwise root the path at `/`.
///
/// `GATES_SHOTS_DIR` wins over everything and is taken **verbatim** — no
/// `gates` component appended. It is the override for a box whose home is not
/// where its pictures are, and for anything driving this client from a script
/// that wants the files somewhere it already knows.
pub fn dir_from(os: HostOs, get: impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let var = |k: &str| get(k).filter(|v| !v.is_empty());
    if let Some(d) = var("GATES_SHOTS_DIR") {
        return Some(PathBuf::from(d));
    }
    let pictures = match os {
        HostOs::Windows => PathBuf::from(var("USERPROFILE")?).join("Pictures"),
        HostOs::Mac => PathBuf::from(var("HOME")?).join("Pictures"),
        // `XDG_PICTURES_DIR` is a user-dirs value rather than a spec'd
        // environment variable, so it is usually absent — but a session that
        // does export it has said where pictures go, and ignoring that to
        // hardcode `~/Pictures` would be this client disagreeing with the
        // desktop on the desktop's own question.
        HostOs::Unix => match var("XDG_PICTURES_DIR") {
            Some(x) => PathBuf::from(x),
            None => PathBuf::from(var("HOME")?).join("Pictures"),
        },
    };
    Some(pictures.join("gates"))
}

/// `YYYYMMDD-HHMMSS`, UTC, from seconds since the Unix epoch.
///
/// Hand-rolled rather than a date crate for `config`'s reason: this is one
/// civil-date conversion, and the client links no calendar today (check
/// `Cargo.toml` before this claim rots).
pub fn stamp(secs: u64) -> String {
    let days = secs / 86_400;
    let rem = secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}{m:02}{d:02}-{:02}{:02}{:02}",
        rem / 3_600,
        (rem % 3_600) / 60,
        rem % 60
    )
}

/// `YYYY-MM-DDTHH:MM:SSZ` — the same instant [`stamp`] names, in the form a
/// document is read in rather than the form a filename needs.
///
/// Here rather than in `crate::report` so this crate holds exactly one civil-
/// date implementation. Two would be two things to keep right, and the one
/// that is wrong would be the one nobody runs — which is this repo's standing
/// complaint about hand-kept mirrors, applied before the mirror exists.
pub fn iso(secs: u64) -> String {
    let days = secs / 86_400;
    let rem = secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3_600,
        (rem % 3_600) / 60,
        rem % 60
    )
}

/// Days since the Unix epoch to `(year, month, day)`, UTC.
///
/// Howard Hinnant's `civil_from_days`, in the unsigned form the epoch allows:
/// the era arithmetic below is exact for every day after 1970-01-01, which is
/// every day this function is ever handed. Integer throughout — no float, so
/// no rounding to reason about.
fn civil_from_days(days: u64) -> (u64, u64, u64) {
    // Shift the era to start on 0000-03-01, which is what makes the leap day
    // the LAST day of a year and the month lengths a repeating pattern.
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z % 146_097; // day of era, [0, 146096]
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // March-based month, [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let y = yoe + era * 400 + u64::from(m <= 2); // January and February are the next year
    (y, m, d)
}

/// This box's clock, in seconds since the epoch, or 0 if it is set before
/// 1970. Not in the sim and not on a gate — [`stamp`] names a file with it and
/// nothing else reads it, so a wrong clock costs a wrong filename and no
/// determinism (`CLAUDE.md` wall 1 is about `sim-core`, which this is not).
pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The first free name for this stamp, or `None` when [`PER_STAMP`] is spent.
///
/// The first shot of a second is `gates-<stamp>.png` and the rest are
/// `-2`, `-3` … — so the common case (one shot, one second) has no suffix at
/// all and the collision handling is invisible until it is needed.
///
/// `exists` is injected so the whole rule is testable without a filesystem.
pub fn pick(dir: &Path, stamp: &str, exists: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    for n in 1..=PER_STAMP {
        let name = if n == 1 {
            format!("{PREFIX}{stamp}.png")
        } else {
            format!("{PREFIX}{stamp}-{n}.png")
        };
        let path = dir.join(name);
        if !exists(&path) {
            return Some(path);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |k| {
            pairs
                .iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| v.to_string())
        }
    }

    #[test]
    fn each_platform_puts_them_where_that_platform_keeps_pictures() {
        assert_eq!(
            dir_from(HostOs::Unix, env(&[("HOME", "/h")])),
            Some(PathBuf::from("/h/Pictures/gates"))
        );
        assert_eq!(
            dir_from(HostOs::Mac, env(&[("HOME", "/Users/x")])),
            Some(PathBuf::from("/Users/x/Pictures/gates"))
        );
        // Forward slashes in the fixture, as `config.rs`'s own Windows case
        // does it: the separator `join` writes is the host's business and
        // this test is about the components.
        assert_eq!(
            dir_from(HostOs::Windows, env(&[("USERPROFILE", "C:/Users/x")])),
            Some(PathBuf::from("C:/Users/x/Pictures/gates"))
        );
    }

    #[test]
    fn a_session_that_says_where_pictures_go_is_believed() {
        // The desktop's own answer wins over `~/Pictures`, on Unix only —
        // it is an XDG user-dirs value and means nothing on the other two.
        assert_eq!(
            dir_from(
                HostOs::Unix,
                env(&[("XDG_PICTURES_DIR", "/p"), ("HOME", "/h")])
            ),
            Some(PathBuf::from("/p/gates"))
        );
        assert_eq!(
            dir_from(
                HostOs::Mac,
                env(&[("XDG_PICTURES_DIR", "/p"), ("HOME", "/h")])
            ),
            Some(PathBuf::from("/h/Pictures/gates"))
        );
    }

    #[test]
    fn the_override_wins_everywhere_and_is_taken_verbatim() {
        // No `gates` component appended: the operator named a directory, and
        // a script that reads the files back must find them where it said.
        for os in [HostOs::Unix, HostOs::Mac, HostOs::Windows] {
            assert_eq!(
                dir_from(
                    os,
                    env(&[
                        ("GATES_SHOTS_DIR", "/shots"),
                        ("HOME", "/h"),
                        ("USERPROFILE", "C:/Users/x"),
                        ("XDG_PICTURES_DIR", "/p"),
                    ])
                ),
                Some(PathBuf::from("/shots"))
            );
        }
    }

    #[test]
    fn an_empty_variable_is_unset() {
        // The XDG rule, applied to all of them: an empty `HOME` would root
        // the path at `/`, and an empty override would name the current
        // directory without anyone asking for it.
        assert_eq!(dir_from(HostOs::Unix, env(&[("HOME", "")])), None);
        assert_eq!(dir_from(HostOs::Mac, env(&[("HOME", "")])), None);
        assert_eq!(dir_from(HostOs::Windows, env(&[("USERPROFILE", "")])), None);
        assert_eq!(
            dir_from(
                HostOs::Unix,
                env(&[("GATES_SHOTS_DIR", ""), ("HOME", "/h")])
            ),
            Some(PathBuf::from("/h/Pictures/gates"))
        );
        assert_eq!(
            dir_from(
                HostOs::Unix,
                env(&[("XDG_PICTURES_DIR", ""), ("HOME", "/h")])
            ),
            Some(PathBuf::from("/h/Pictures/gates"))
        );
    }

    #[test]
    fn no_base_is_no_screenshots_rather_than_a_file_beside_the_binary() {
        assert_eq!(dir_from(HostOs::Unix, env(&[])), None);
        assert_eq!(dir_from(HostOs::Mac, env(&[])), None);
        assert_eq!(dir_from(HostOs::Windows, env(&[])), None);
    }

    #[test]
    fn the_stamp_is_utc_and_sorts_as_it_reads() {
        assert_eq!(stamp(0), "19700101-000000");
        // 2026-08-15T14:12:03Z — the day this landed, as a fixed point.
        assert_eq!(stamp(1_786_803_123), "20260815-141203");
        // A leap day, which the era arithmetic exists to get right.
        assert_eq!(stamp(1_709_164_800), "20240229-000000");
        // The century rule, both ways — the half a hand-rolled calendar gets
        // wrong. 2000 is divisible by 400 and HAS a 29th; 2100 is divisible
        // by 100 and does not, so its day 58 is the 28th and day 59 is March.
        assert_eq!(stamp(951_782_400), "20000229-000000");
        assert_eq!(stamp(4_107_456_000), "21000228-000000");
        assert_eq!(stamp(4_107_542_400), "21000301-000000");

        // Lexicographic order is chronological order — the whole point of the
        // layout, and what makes a file manager's name sort useful.
        let secs = [1_786_803_123u64, 0, 1_709_164_800, 951_782_400];
        let mut names: Vec<String> = secs.iter().map(|s| stamp(*s)).collect();
        names.sort();
        let mut chronological = secs;
        chronological.sort();
        let by_time: Vec<String> = chronological.iter().map(|s| stamp(*s)).collect();
        assert_eq!(names, by_time);
    }

    #[test]
    fn every_second_of_a_day_round_trips_to_a_distinct_name() {
        // A whole day at one-second steps: 86,400 stamps, all distinct and
        // all monotonic. Cheap, and it is the property the filename rests on.
        let mut seen = HashSet::new();
        let base = 1_786_752_000; // 2026-08-15T00:00:00Z
        let mut last = String::new();
        for s in 0..86_400u64 {
            let st = stamp(base + s);
            assert!(st > last, "{st} !> {last}");
            last = st.clone();
            assert!(seen.insert(st), "duplicate stamp at +{s}");
        }
        assert_eq!(seen.len(), 86_400);
        assert_eq!(stamp(base), "20260815-000000");
        assert_eq!(stamp(base + 86_399), "20260815-235959");
    }

    #[test]
    fn the_first_shot_of_a_second_has_no_suffix_and_the_rest_count_up() {
        let dir = Path::new("/shots");
        let taken: HashSet<PathBuf> = HashSet::new();
        assert_eq!(
            pick(dir, "20260815-141203", |p| taken.contains(p)),
            Some(PathBuf::from("/shots/gates-20260815-141203.png"))
        );

        let mut taken = taken;
        taken.insert(PathBuf::from("/shots/gates-20260815-141203.png"));
        assert_eq!(
            pick(dir, "20260815-141203", |p| taken.contains(p)),
            Some(PathBuf::from("/shots/gates-20260815-141203-2.png"))
        );
        taken.insert(PathBuf::from("/shots/gates-20260815-141203-2.png"));
        assert_eq!(
            pick(dir, "20260815-141203", |p| taken.contains(p)),
            Some(PathBuf::from("/shots/gates-20260815-141203-3.png"))
        );
    }

    #[test]
    fn the_stamp_is_bounded_and_says_so() {
        // Wall 4: a player-driven path needs a cap and a stated policy. Every
        // name for this second is spent, so the answer is None — refuse and
        // say so — rather than a `stat` loop with no end.
        let dir = Path::new("/shots");
        assert_eq!(pick(dir, "20260815-141203", |_| true), None);
        // And the cap is exactly `PER_STAMP` names, not one more or less.
        let mut taken: HashSet<PathBuf> = HashSet::new();
        let mut n = 0;
        while let Some(p) = pick(dir, "s", |p| taken.contains(p)) {
            assert!(taken.insert(p), "pick handed back a name it had already");
            n += 1;
            assert!(n <= PER_STAMP, "pick never gave up");
        }
        assert_eq!(n, PER_STAMP);
    }
}
