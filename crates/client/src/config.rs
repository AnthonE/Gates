//! The client's settings file — the path, the format, and what happens to a
//! key this build does not recognize (`DECISIONS.md` §open, "settings v0").
//!
//! **Format: hand-rolled `key = value` TOML**, the shape
//! `server/src/config.rs` already parses for `shard.toml` and for the same
//! stated reason — eight keys don't earn a serde dependency, and the client
//! links no TOML parser today (check `Cargo.toml` before this claim rots).
//! One deliberate difference from the server's parser: `shard.toml` REFUSES
//! an unknown key, because an operator's typo silently running defaults is
//! the failure there. This file is written by the game itself, so an unknown
//! key is almost never a typo — it is another build's knob, and the policy is
//! **ignore-and-preserve**: the reader skips what it does not know and the
//! writer puts it back verbatim, so launching an older build does not strip
//! the settings a newer one saved. That is the choice that loses least, and
//! it is why reading a file whose `version` is higher than ours is safe.
//!
//! **Version**: `SETTINGS_VERSION`, written into every file. It exists for
//! the day a knob is RENAMED — the loader that renames `old_key` to
//! `new_key` keys the migration off the file's version, because without one
//! it cannot tell "old file, old meaning" from "hand-typed stray". Adding or
//! removing a key needs no bump (missing keys default, unknown keys are
//! preserved); only a rename or a change of meaning does. The writer stamps
//! `max(file version, ours)` so the stamp never walks backwards over keys it
//! cannot read.
//!
//! **Path: the platform's config dir, resolved from the environment by
//! hand** — `%APPDATA%\gates\` on Windows, `~/Library/Application Support/
//! gates/` on macOS, `$XDG_CONFIG_HOME/gates/` (or `~/.config/gates/`)
//! elsewhere. Hand-rolled rather than a `dirs`-style crate because this is
//! three env reads, and a `gates.toml` beside the binary would be dishonest
//! for an installed game — the depot dir is the launcher's to replace
//! wholesale on update. No resolvable base (no `HOME`) means no persistence
//! for the run, never an error.
//!
//! **A missing or corrupt file is the defaults, silently.** A fresh boot
//! must not error, and the damage is bounded either way: a line that does
//! not parse costs that line its value, never the file. The write side is
//! temp-file-and-rename so a crash mid-write cannot leave a torn file that
//! would then "silently be the defaults" at the next boot.
//!
//! **Not feature-gated**, for `ui`'s reason exactly: parse and serialize are
//! pure arithmetic, and a test behind `--features render` runs in the
//! renderer tier where nobody looks at it. The Bevy half — load once at
//! startup, save on change — is `render/settings.rs`.

use std::path::{Path, PathBuf};

/// Bump ONLY when a key is renamed or changes meaning — see the module
/// header. `DECISIONS.md` §open, "settings v0".
pub const SETTINGS_VERSION: u32 = 1;

/// The eight values that survive a restart. A plain struct rather than the
/// render module's `Settings` because that type also carries UI state (the
/// selected rail row, where Esc returns to) that has no business on disk —
/// and because this module must build without Bevy.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Persisted {
    pub fov_deg: f32,
    pub sensitivity: f32,
    pub invert_look: bool,
    pub vsync: bool,
    /// Frames per second ceiling; **0 means uncapped**, and then vsync is the
    /// only thing holding the loop down.
    ///
    /// Bevy's focused update mode is `Continuous` — as fast as it possibly
    /// can — so with vsync off there was nothing between the render loop and
    /// the hardware. A menu with nothing in it would spin a core and a GPU at
    /// four figures to draw the same still frame. (Its *unfocused* mode is
    /// already `reactive_low_power(1/60)`, so a backgrounded client was never
    /// the problem; a focused one on a menu was.)
    pub max_fps: u16,
    pub fullscreen: bool,
    pub vol_master: f32,
    pub vol_game: f32,
    pub vol_ambience: f32,
    pub vol_music: f32,
    /// Tell Discord what the player is doing (`crate::discord`). On by
    /// default, and that is safe rather than presumptuous for two reasons:
    /// the whole path is dark unless the build carries an application id,
    /// and what it says at this level — the verb, the place, the party
    /// count — locates no machine and names no person.
    pub discord_presence: bool,
    /// Put the shard's name and address in that presence, which is what
    /// makes Discord's **Ask to Join** appear.
    ///
    /// **Off by default, and it is the one setting here that is a
    /// disclosure rather than a preference.** A presence line is visible to
    /// everyone who can see the profile, so this hands strangers the address
    /// of the box the player is on. The operator opened the door
    /// (2026-08-16, *"nothing wrong with that if the player enables it"*) —
    /// opt-in is what "enables it" means, and `reference/VOICE.md` §9.1 is
    /// what the other default costs.
    pub discord_share_server: bool,
}

/// What a parse hands back: the values (defaults where the file was silent
/// or wrong), the file's version, and every unknown `key = value` line kept
/// verbatim for the next save.
#[derive(Debug)]
pub struct Loaded {
    pub values: Persisted,
    pub version: u32,
    pub unknown: Vec<String>,
    /// The starred shard ids, in file order.
    ///
    /// **Not on [`Persisted`]**, which is `Copy` and eight scalars, and would
    /// stop being either. It is also not the same *kind* of thing: every field
    /// up there is a knob with a range, and this is a list whose bound and
    /// whose deduplication live beside the star that writes it
    /// (`crate::ui::servers::Favourites::from_disk`) for the same reason the
    /// numeric clamps live beside the steppers — two sanitizers drift.
    pub favourites: Vec<String>,
}

/// Parse settings text over `defaults`. Never fails: a line that does not
/// parse contributes nothing, a known key with a bad or non-finite value
/// keeps its default (and is NOT preserved — the next save rewrites it),
/// and an unknown key that is at least `key = value` shaped is preserved.
/// Range clamping is deliberately not here — the bounds live beside the
/// steppers in `render/settings.rs`, and two clamps drift.
pub fn parse(text: &str, defaults: Persisted) -> Loaded {
    let mut v = defaults;
    let mut version = SETTINGS_VERSION;
    let mut favourites: Vec<String> = Vec::new();
    // (key, full line) so a duplicated unknown key keeps only its last value
    // — the same last-wins the known keys get from being plain assignments.
    let mut unknown: Vec<(String, String)> = Vec::new();
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            // Not even `key = value`. Nothing to preserve: our writer never
            // produces such a line, so keeping it would grow junk forever.
            continue;
        };
        let key = key.trim();
        let raw = value.trim();
        let value = raw.trim_matches('"');
        match key {
            "version" => {
                if let Ok(n) = value.parse() {
                    version = n;
                }
            }
            "fov_deg" => num(&mut v.fov_deg, value),
            "sensitivity" => num(&mut v.sensitivity, value),
            "invert_look" => flag(&mut v.invert_look, value),
            "vsync" => flag(&mut v.vsync, value),
            "max_fps" => count(&mut v.max_fps, value),
            "fullscreen" => flag(&mut v.fullscreen, value),
            "vol_master" => num(&mut v.vol_master, value),
            "vol_game" => num(&mut v.vol_game, value),
            "vol_ambience" => num(&mut v.vol_ambience, value),
            "vol_music" => num(&mut v.vol_music, value),
            "discord_presence" => flag(&mut v.discord_presence, value),
            "discord_share_server" => flag(&mut v.discord_share_server, value),
            // A comma-separated list, because the format is `key = value` and
            // a list of ids does not earn a second one. An id may not contain
            // a comma — `shardlist::parse` caps every field at
            // `MAX_FIELD_BYTES` and this file is written from ids that came
            // through it — so the split is lossless for anything the game
            // itself wrote, and a hand-typed comma costs that entry its star
            // rather than the file its meaning.
            "favourites" => {
                favourites = value
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect();
            }
            _ => {
                if key_shaped(key) {
                    unknown.retain(|(k, _)| k != key);
                    unknown.push((key.to_string(), format!("{key} = {raw}")));
                }
            }
        }
    }
    Loaded {
        values: v,
        version,
        unknown: unknown.into_iter().map(|(_, line)| line).collect(),
        favourites,
    }
}

/// A finite number or nothing. `NaN.clamp()` is NaN, so the finite check has
/// to happen here where the value first exists — a NaN that reached the
/// renderer would be a fov the projection cannot build.
fn num(slot: &mut f32, value: &str) {
    if let Ok(n) = value.parse::<f32>() {
        if n.is_finite() {
            *slot = n;
        }
    }
}

/// TOML booleans only. Anything else keeps the default rather than guessing
/// what "yes" meant.
fn flag(slot: &mut bool, value: &str) {
    match value {
        "true" => *slot = true,
        "false" => *slot = false,
        _ => {}
    }
}

/// A whole non-negative count. Out-of-range or non-numeric keeps the default
/// rather than guessing — `num`'s posture for a field that is not a float.
fn count(slot: &mut u16, value: &str) {
    if let Ok(n) = value.parse::<u16>() {
        *slot = n;
    }
}

/// Is this something a future build could plausibly have written as a key?
/// ASCII word characters only — the preserve policy is for another version's
/// knobs, not for arbitrary bytes to ride the file forever.
fn key_shaped(key: &str) -> bool {
    !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The file, as written. `version` is the loaded file's — the writer stamps
/// whichever of that and `SETTINGS_VERSION` is higher, so saving from an
/// older build under a newer file never claims the preserved keys are older
/// than they are.
pub fn serialize(v: &Persisted, version: u32, favourites: &[String], unknown: &[String]) -> String {
    let mut s = String::with_capacity(512);
    s.push_str("# gates client settings. Written by the game on every change;\n");
    s.push_str("# hand edits are read at the next launch. A key this build does\n");
    s.push_str("# not know is kept as it is, not dropped.\n");
    s.push_str(&format!("version = {}\n", version.max(SETTINGS_VERSION)));
    s.push_str(&format!("fov_deg = {}\n", v.fov_deg));
    s.push_str(&format!("sensitivity = {}\n", v.sensitivity));
    s.push_str(&format!("invert_look = {}\n", v.invert_look));
    s.push_str(&format!("vsync = {}\n", v.vsync));
    s.push_str(&format!("max_fps = {}\n", v.max_fps));
    s.push_str(&format!("fullscreen = {}\n", v.fullscreen));
    s.push_str(&format!("vol_master = {}\n", v.vol_master));
    s.push_str(&format!("vol_game = {}\n", v.vol_game));
    s.push_str(&format!("vol_ambience = {}\n", v.vol_ambience));
    s.push_str(&format!("vol_music = {}\n", v.vol_music));
    s.push_str(&format!("discord_presence = {}\n", v.discord_presence));
    s.push_str(&format!(
        "discord_share_server = {}\n",
        v.discord_share_server
    ));
    // Written unconditionally, empty list included: a `favourites = ""` line
    // is how un-starring the last shard *sticks*. Omitting the key when the
    // list is empty would leave the previous file's line in place on a
    // rewrite, and the star would come back at the next launch.
    s.push_str(&format!("favourites = \"{}\"\n", favourites.join(",")));
    for line in unknown {
        s.push_str(line);
        s.push('\n');
    }
    s
}

/// Read the file at `path`, or the defaults if there is nothing readable
/// there — a fresh boot must not error, and must not write either (no file
/// appears until the player changes something).
pub fn load(path: &Path, defaults: Persisted) -> Loaded {
    match std::fs::read_to_string(path) {
        Ok(text) => parse(&text, defaults),
        Err(_) => Loaded {
            values: defaults,
            version: SETTINGS_VERSION,
            unknown: Vec::new(),
            favourites: Vec::new(),
        },
    }
}

/// Write `text` to `path`, creating the directory, via temp-file-and-rename
/// so a crash mid-write leaves the old file whole rather than a torn one.
pub fn save(path: &Path, text: &str) -> Result<(), String> {
    let dir = path
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", path.display()))?;
    std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("rename to {}: {e}", path.display()))
}

/// Which platform's convention [`settings_path`] resolves. A value rather
/// than three `#[cfg]`'d function bodies so every branch compiles and tests
/// on every box — a path rule that only its own OS can check is a path rule
/// CI never checks.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HostOs {
    Windows,
    Mac,
    Unix,
}

#[cfg(target_os = "windows")]
pub const HOST_OS: HostOs = HostOs::Windows;
#[cfg(target_os = "macos")]
pub const HOST_OS: HostOs = HostOs::Mac;
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub const HOST_OS: HostOs = HostOs::Unix;

/// Where the settings file lives on this box, or `None` when the
/// environment names no base to put a config dir in — which disables
/// persistence for the run rather than erroring or writing beside the
/// binary (the depot dir is the launcher's to replace wholesale on update).
pub fn settings_path() -> Option<PathBuf> {
    path_from(HOST_OS, |k| std::env::var(k).ok())
}

/// The resolution rule, with the environment injected so the three branches
/// are testable without touching the process's real (and test-shared) env.
/// An empty variable counts as unset — the XDG spec's own rule, applied to
/// all of them because an empty `HOME` would otherwise root the path at `/`.
pub fn path_from(os: HostOs, get: impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let var = |k: &str| get(k).filter(|v| !v.is_empty());
    let base = match os {
        HostOs::Windows => PathBuf::from(var("APPDATA")?),
        HostOs::Mac => PathBuf::from(var("HOME")?).join("Library/Application Support"),
        HostOs::Unix => match var("XDG_CONFIG_HOME") {
            Some(x) => PathBuf::from(x),
            None => PathBuf::from(var("HOME")?).join(".config"),
        },
    };
    Some(base.join("gates").join("settings.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defaults() -> Persisted {
        // Test-local, deliberately NOT the shipped defaults: these tests are
        // about overlay mechanics, and a fixture equal to the real defaults
        // could pass with a parser that reads nothing. The shipped defaults
        // are owned by `render/settings.rs` and asserted there.
        Persisted {
            fov_deg: 75.0,
            sensitivity: 1.0,
            invert_look: false,
            vsync: true,
            max_fps: 60,
            fullscreen: false,
            vol_master: 1.0,
            vol_game: 1.0,
            vol_ambience: 1.0,
            vol_music: 1.0,
            discord_presence: false,
            discord_share_server: true,
        }
    }

    fn changed() -> Persisted {
        Persisted {
            fov_deg: 90.0,
            sensitivity: 0.55,
            invert_look: true,
            vsync: false,
            max_fps: 0,
            fullscreen: true,
            vol_master: 0.7,
            vol_game: 0.3,
            vol_ambience: 0.0,
            vol_music: 0.45,
            discord_presence: true,
            discord_share_server: false,
        }
    }

    #[test]
    fn a_round_trip_is_identity() {
        let unknown = vec!["future_knob = 3".to_string()];
        let text = serialize(&changed(), SETTINGS_VERSION, &[], &unknown);
        let back = parse(&text, defaults());
        assert_eq!(back.values, changed());
        assert_eq!(back.version, SETTINGS_VERSION);
        assert_eq!(back.unknown, unknown);
    }

    #[test]
    fn a_missing_or_garbled_file_is_the_defaults() {
        // Empty text, no file shape at all, and a binary smear: every one is
        // the defaults, silently — a fresh boot must not error.
        assert_eq!(parse("", defaults()).values, defaults());
        assert_eq!(parse("not a settings file", defaults()).values, defaults());
        assert_eq!(
            parse("\u{0}\u{1}== = ==\n===", defaults()).values,
            defaults()
        );
    }

    #[test]
    fn a_bad_value_on_a_known_key_costs_that_key_only() {
        let text = "fov_deg = squirrel\nsensitivity = NaN\nvol_master = inf\nvsync = maybe\nvol_game = 0.3\n";
        let got = parse(text, defaults()).values;
        // The bad ones kept their defaults — including the two non-finite
        // floats, which parse::<f32> accepts and the renderer must never see.
        assert_eq!(got.fov_deg, defaults().fov_deg);
        assert_eq!(got.sensitivity, defaults().sensitivity);
        assert_eq!(got.vol_master, defaults().vol_master);
        assert_eq!(got.vsync, defaults().vsync);
        // The good line beside them still landed: the file is not all-or-nothing.
        assert_eq!(got.vol_game, 0.3);
    }

    #[test]
    fn an_unknown_key_survives_a_save() {
        // The ignore-and-preserve policy: an older build must not strip a
        // newer build's knob. Comments and malformed lines do NOT survive —
        // preserved keys are the policy, junk riding forever is not.
        let text = "vol_game = 0.5\nfuture_knob = 3\n# a comment\nnot a line\nfuture_knob = 4\n";
        let got = parse(text, defaults());
        assert_eq!(got.unknown, vec!["future_knob = 4".to_string()]);
        let out = serialize(&got.values, got.version, &got.favourites, &got.unknown);
        assert!(out.contains("future_knob = 4"), "{out}");
        assert!(!out.contains("not a line"), "{out}");
    }

    #[test]
    fn a_newer_file_is_read_and_its_version_kept() {
        // Reading forward is safe BECAUSE of ignore-and-preserve, and the
        // stamp never walks backwards over keys this build cannot read.
        let text = "version = 9\nfov_deg = 80\nrenamed_knob = 1\n";
        let got = parse(text, defaults());
        assert_eq!(got.version, 9);
        assert_eq!(got.values.fov_deg, 80.0);
        let out = serialize(&got.values, got.version, &got.favourites, &got.unknown);
        assert!(out.contains("version = 9"), "{out}");
    }

    #[test]
    fn the_file_round_trips_through_disk() {
        // The one fs test: save creates the directory, load reads back what
        // was saved, and a missing file is the defaults.
        let dir = std::env::temp_dir().join(format!("gates-settings-test-{}", std::process::id()));
        let path = dir.join("deeper").join("settings.toml");
        assert_eq!(load(&path, defaults()).values, defaults());
        save(&path, &serialize(&changed(), SETTINGS_VERSION, &[], &[])).expect("save");
        assert_eq!(load(&path, defaults()).values, changed());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_path_is_the_platform_convention() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |k: &str| {
                pairs
                    .iter()
                    .find(|(name, _)| *name == k)
                    .map(|(_, v)| v.to_string())
            }
        };
        let p = |s: &str| Some(PathBuf::from(s));
        assert_eq!(
            path_from(
                HostOs::Unix,
                env(&[("XDG_CONFIG_HOME", "/xdg"), ("HOME", "/h")])
            ),
            p("/xdg/gates/settings.toml")
        );
        assert_eq!(
            path_from(HostOs::Unix, env(&[("HOME", "/h")])),
            p("/h/.config/gates/settings.toml")
        );
        // An empty variable is unset (the XDG rule, applied to all three).
        assert_eq!(
            path_from(
                HostOs::Unix,
                env(&[("XDG_CONFIG_HOME", ""), ("HOME", "/h")])
            ),
            p("/h/.config/gates/settings.toml")
        );
        assert_eq!(path_from(HostOs::Unix, env(&[("HOME", "")])), None);
        assert_eq!(
            path_from(HostOs::Mac, env(&[("HOME", "/Users/x")])),
            p("/Users/x/Library/Application Support/gates/settings.toml")
        );
        assert_eq!(
            path_from(
                HostOs::Windows,
                env(&[("APPDATA", "C:/Users/x/AppData/Roaming")])
            ),
            p("C:/Users/x/AppData/Roaming/gates/settings.toml")
        );
        // No base anywhere: persistence is off for the run, never an error.
        assert_eq!(path_from(HostOs::Windows, env(&[])), None);
    }
}
