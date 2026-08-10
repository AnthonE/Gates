//! Settings — the reference's options screen, at the size ours can be honest
//! about.
//!
//! **The shape is the reference's**: a category rail down the left with the
//! selected one blocked out in olive, a pane of rows on the right, each row a
//! label and a control. That is the frame the operator handed over, and it is
//! worth copying because it scales — the pane holds two rows today and thirty
//! later without the screen changing shape.
//!
//! **What is NOT copied is the row count.** The reference's GRAPHICS tab lists
//! shadow cascades, anisotropic filtering, parallax mapping and a dozen more
//! because it has a renderer with those switches. Ours has five settings that
//! do something, and this screen shows five settings. A category with nothing
//! behind it says so in a sentence instead of drawing greyed rows that imply
//! a feature exists — the same rule the HUD already obeys, where "a 0-max
//! meter is undrawn, not drawn empty", and the same rule the intro screen
//! obeys when it says why the shard list is empty rather than showing nothing.
//!
//! **Settings persist.** `crate::config` owns the file — the path (the
//! platform's config dir), the format (hand-rolled `key = value` TOML, the
//! server's `shard.toml` precedent), the `version` field for when a knob is
//! renamed, and the ignore-and-preserve policy for a key this build does not
//! know. This module owns the Bevy half: [`load`] runs once at plugin build,
//! before the first frame that applies anything, and [`save_on_change`]
//! writes on every real change rather than at exit — a crash must not cost
//! the player their settings, and a change is a click, not a hot path. What
//! comes off disk is sanitized HERE, beside the steppers' own bounds: a
//! hand-edited `fov_deg = 500` is clamped exactly where a clicked one would
//! be, so the file cannot reach a state the screen cannot. The footer names
//! the file, or says when nothing is being saved (a capture run, a box with
//! no config dir) — the screen does not quietly forget on the player's
//! behalf, and it does not quietly claim to remember either.
//!
//! **One thing this screen deliberately cannot do is touch the sim.** Every
//! field below changes how the client draws or how the mouse maps to a view
//! angle. None of them reaches `set_input`'s quantization — a "sensitivity"
//! that changed the wire's yaw resolution would be a client tuning what the
//! server predicts against, which is the quantize-both-sides law's exact
//! failure. Sensitivity scales the free-running radian yaw *before* it is
//! quantized, so both sides still agree bit for bit.

use bevy::prelude::*;
use bevy::window::{PresentMode, PrimaryWindow, WindowMode};

use crate::config::{self, Persisted};
use crate::ui::servers::Favourites;

use super::menu::Screen;
use super::rig::{EyeCam, FOV_DEG};
use super::ui;

/// Bounds and steps for the two numeric settings (`DECISIONS.md` §open,
/// "settings v0"). The defaults themselves are not new numbers: the field of
/// view starts at the rig's shipped `FOV_DEG` and sensitivity starts at 1.0,
/// which is the identity against `input::MOUSE_RAD_PER_PX`.
pub const FOV_MIN_DEG: f32 = 60.0;
pub const FOV_MAX_DEG: f32 = 110.0;
pub const FOV_STEP_DEG: f32 = 5.0;
pub const SENS_MIN: f32 = 0.25;
pub const SENS_MAX: f32 = 3.0;
pub const SENS_STEP: f32 = 0.05;

/// Volume sliders run 0..1 in tenths — the reference's `audio.master`,
/// `audio.game` and `audio.ambience` are 0..1 convars and its options screen
/// is a row of sliders over the same range. A tenth is the smallest step a
/// player can hear as a step; a hundredth would be twenty clicks of nothing.
pub const VOL_STEP: f32 = 0.1;

/// The categories, in the reference's order. Every one is drawn whether or not
/// it has rows — the rail is a map of the game's surface, and a category that
/// vanished when it was empty would make the screen change shape as features
/// land.
pub const CATEGORIES: [&str; 6] = [
    "GAMEPLAY", "AUDIO", "SCREEN", "GRAPHICS", "CONTROLS", "KEYBINDS",
];

/// What the player can change. Client-side every one of them; see the header.
#[derive(Resource)]
pub struct Settings {
    pub fov_deg: f32,
    /// A multiplier on `input::MOUSE_RAD_PER_PX`, not a replacement for it.
    pub sensitivity: f32,
    pub invert_look: bool,
    pub vsync: bool,
    pub fullscreen: bool,
    /// The three audio buses, 0..1. The reference's `audio.master`,
    /// `audio.game` and `audio.ambience` — `crate::sound::Mix` is what reads
    /// them, and `render/audio.rs` is the only thing that builds one.
    pub vol_master: f32,
    pub vol_game: f32,
    pub vol_ambience: f32,
    /// Which rail row is selected.
    pub cat: usize,
    /// Where Esc returns to. Settings is reachable from the intro screen and
    /// from the Esc menu, and it has to go back where it came from — a screen
    /// that always returned to the menu would drop a player out of a live
    /// world for changing their field of view.
    pub back: Screen,
    /// Set when the pane has to be rebuilt. The same explicit flag `Menu`
    /// carries, for the same reason: `is_changed()` fires on the frame the
    /// resource is inserted, which would rebuild what `setup` just spawned.
    pub dirty: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            fov_deg: FOV_DEG,
            sensitivity: 1.0,
            invert_look: false,
            vsync: true,
            fullscreen: false,
            // The reference opens master and game at 1. Ours opens the bed
            // lower than either of them, but that belongs to the cue's own
            // gain rather than to a bus a player would then have to put back.
            vol_master: 1.0,
            vol_game: 1.0,
            vol_ambience: 1.0,
            cat: 0,
            back: Screen::Menu,
            dirty: false,
        }
    }
}

/// Every setting a control can move, as one enum, so the click handler is a
/// match rather than five queries.
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Knob {
    Vsync,
    Fullscreen,
    Fov,
    Sensitivity,
    InvertLook,
    VolMaster,
    VolGame,
    VolAmbience,
}

impl Settings {
    /// Move a setting. `delta` is +1/-1 for a numeric row and ignored by a
    /// toggle, which flips.
    ///
    /// Clamping lives here rather than in the click handler so the bounds are
    /// testable without a window, and so a keyboard path added later cannot
    /// reach a different clamp than the mouse path.
    pub fn adjust(&mut self, knob: Knob, delta: i32) {
        match knob {
            Knob::Vsync => self.vsync = !self.vsync,
            Knob::Fullscreen => self.fullscreen = !self.fullscreen,
            Knob::InvertLook => self.invert_look = !self.invert_look,
            Knob::Fov => {
                self.fov_deg =
                    (self.fov_deg + delta as f32 * FOV_STEP_DEG).clamp(FOV_MIN_DEG, FOV_MAX_DEG);
            }
            Knob::Sensitivity => {
                // Rounded to the step, not just clamped: repeated float adds
                // of 0.05 drift, and a screen that reads "1.00" while holding
                // 0.9999997 would eventually print "0.95" for a click of +.
                let steps = ((self.sensitivity + delta as f32 * SENS_STEP) / SENS_STEP).round();
                self.sensitivity = (steps * SENS_STEP).clamp(SENS_MIN, SENS_MAX);
            }
            // Same round-to-step as sensitivity, for the same reason: a
            // slider that reads "70%" while holding 0.6999998 eventually
            // prints the wrong tenth for a click of +.
            Knob::VolMaster => self.vol_master = step_vol(self.vol_master, delta),
            Knob::VolGame => self.vol_game = step_vol(self.vol_game, delta),
            Knob::VolAmbience => self.vol_ambience = step_vol(self.vol_ambience, delta),
        }
        self.dirty = true;
    }

    /// What the control on this row currently reads.
    fn value(&self, knob: Knob) -> String {
        match knob {
            Knob::Vsync => on_off(self.vsync),
            Knob::Fullscreen => on_off(self.fullscreen),
            Knob::InvertLook => on_off(self.invert_look),
            Knob::Fov => format!("{:.0}", self.fov_deg),
            Knob::Sensitivity => format!("{:.2}", self.sensitivity),
            Knob::VolMaster => pct(self.vol_master),
            Knob::VolGame => pct(self.vol_game),
            Knob::VolAmbience => pct(self.vol_ambience),
        }
    }

    /// The eight values that go to disk — everything a player set, none of
    /// the UI state (`cat`, `back`, `dirty` describe this run's screen, not
    /// the player's choices).
    fn persisted(&self) -> Persisted {
        Persisted {
            fov_deg: self.fov_deg,
            sensitivity: self.sensitivity,
            invert_look: self.invert_look,
            vsync: self.vsync,
            fullscreen: self.fullscreen,
            vol_master: self.vol_master,
            vol_game: self.vol_game,
            vol_ambience: self.vol_ambience,
        }
    }

    /// A loaded file, sanitized against the SAME bounds the steppers
    /// enforce. `config::parse` already refused non-finite floats; the range
    /// and the step live here, beside the controls, so a hand-edited file
    /// cannot reach a value a hundred clicks cannot — `fov_deg = 500` loads
    /// as 110, and an off-step sensitivity is rounded exactly as a click
    /// would round it, because the screen prints the step's precision.
    fn from_persisted(p: Persisted) -> Self {
        let step = |v: f32, s: f32| (v / s).round() * s;
        Self {
            fov_deg: p.fov_deg.clamp(FOV_MIN_DEG, FOV_MAX_DEG),
            sensitivity: step(p.sensitivity, SENS_STEP).clamp(SENS_MIN, SENS_MAX),
            invert_look: p.invert_look,
            vsync: p.vsync,
            fullscreen: p.fullscreen,
            vol_master: step(p.vol_master, VOL_STEP).clamp(0.0, 1.0),
            vol_game: step(p.vol_game, VOL_STEP).clamp(0.0, 1.0),
            vol_ambience: step(p.vol_ambience, VOL_STEP).clamp(0.0, 1.0),
            ..Self::default()
        }
    }
}

/// What is on the disk right now, held so [`save_on_change`] can tell a real
/// change from a rebuild flag — `dirty` is also set by picking a rail row,
/// which changes nothing worth a write. Only inserted when there is a file
/// to keep: a capture run never gets one (frames must not depend on the
/// box's config, so it loads nothing and saves nothing), and neither does a
/// box whose environment names no config dir.
#[derive(Resource)]
pub struct Disk {
    path: std::path::PathBuf,
    /// The loaded file's version — carried so a save under a newer file
    /// keeps its stamp (`config::serialize` takes the max).
    version: u32,
    /// Another build's keys, preserved verbatim (`crate::config`'s policy).
    unknown: Vec<String>,
    /// The sanitized view of what was loaded, NOT the raw file: an
    /// out-of-range value on disk is corrected in memory at load and on disk
    /// only at the player's next change — a boot must not write.
    written: Persisted,
    /// The starred shards as last written. Compared the same way `written`
    /// is, and for the same reason: a star is a click on a different screen
    /// entirely, so this file has two writers' worth of state in it and
    /// exactly one writer.
    written_favs: Vec<String>,
}

/// Read the settings file once, before the first frame that applies
/// anything. Missing or corrupt is the defaults, silently; no resolvable
/// path is the defaults with persistence off for the run (`None`).
pub fn load() -> (Settings, Favourites, Option<Disk>) {
    let Some(path) = config::settings_path() else {
        return (Settings::default(), Favourites::default(), None);
    };
    let loaded = config::load(&path, Settings::default().persisted());
    let settings = Settings::from_persisted(loaded.values);
    // Bounded and deduplicated where the star is, not where the file is —
    // `config::parse`'s own header says range work does not live there, and a
    // hand-edited list of a thousand ids must reach the browser as the same
    // thing a thousand clicks could have produced.
    let favs = Favourites::from_disk(loaded.favourites);
    let disk = Disk {
        path,
        version: loaded.version,
        unknown: loaded.unknown,
        written: settings.persisted(),
        written_favs: favs.ids().to_vec(),
    };
    (settings, favs, Some(disk))
}

/// Save on change, not on exit: a crash must not cost the player their
/// settings, and a change is a click on a menu screen, not a hot path — the
/// write is a few hundred bytes behind a change-detection early-out and a
/// field compare, so an idle frame pays one branch.
/// **One writer, two watched resources.** The settings screen owns eight
/// knobs and the server browser owns the favourite list, and both live in one
/// file — so the alternative was two systems serializing the whole document,
/// which is the shape where the last one to run silently drops the other's
/// change. `Browse` is read here rather than `Settings` growing a list,
/// because a favourite is not a knob and `Settings::adjust` has no arm that
/// could take one.
pub fn save_on_change(
    settings: Res<Settings>,
    browse: Res<super::menu::Browse>,
    disk: Option<ResMut<Disk>>,
) {
    let Some(mut disk) = disk else {
        return;
    };
    if !settings.is_changed() && !browse.is_changed() {
        return;
    }
    let now = settings.persisted();
    let favs = browse.favourites.ids();
    if now == disk.written && favs == disk.written_favs {
        return;
    }
    let text = config::serialize(&now, disk.version, favs, &disk.unknown);
    match config::save(&disk.path, &text) {
        Ok(()) => {
            disk.written = now;
            disk.written_favs = favs.to_vec();
        }
        // Warn and keep `written` as it was, so the next change retries.
        // Never a panic and never a dialog: a full disk must not cost the
        // player the fov they just picked, only its survival.
        Err(e) => warn!("gates: settings not saved - {e}"),
    }
}

/// One click of a volume slider, rounded onto the step and clamped to 0..1.
fn step_vol(v: f32, delta: i32) -> f32 {
    let steps = ((v + delta as f32 * VOL_STEP) / VOL_STEP).round();
    (steps * VOL_STEP).clamp(0.0, 1.0)
}

fn pct(v: f32) -> String {
    format!("{:.0}%", v * 100.0)
}

fn on_off(b: bool) -> String {
    if b {
        "ON".to_string()
    } else {
        "OFF".to_string()
    }
}

/// One row of the pane. Three kinds, because three is what the settings we
/// actually have need — a toggle, a number with steppers, and a fact.
enum Row {
    Toggle(&'static str, Knob),
    Number(&'static str, Knob, &'static str),
    /// A read-only row: a label and a value nobody can change here. The
    /// keybind list is all of these, and so is anything the client fixes.
    Fact(&'static str, &'static str),
    /// A sentence, for a category with nothing in it.
    Note(&'static str),
}

/// The pane's contents, by category. This is the whole of what settings can
/// do, in one readable place.
fn rows(cat: usize) -> Vec<Row> {
    match CATEGORIES[cat] {
        "GAMEPLAY" => vec![Row::Note(
            "No gameplay options exist yet. Every rule this screen could relax is \
             the server's, and a client that could turn one off would be a client \
             deciding - which is the one thing it may never do.",
        )],
        "AUDIO" => vec![
            Row::Number("MASTER VOLUME", Knob::VolMaster, "of full"),
            Row::Number("GAME VOLUME", Knob::VolGame, "steps, tools, hits"),
            Row::Number("AMBIENCE VOLUME", Knob::VolAmbience, "the wind bed"),
            // Two facts rather than two dead sliders, and both are the same
            // rule this screen already obeys: a category with nothing behind
            // it says so. There is no music and no voice chat, so there is no
            // music slider and no voice slider.
            Row::Fact("MUSIC", "None yet - see NOW.md"),
            Row::Fact("VOICE CHAT", "Not implemented"),
            Row::Fact("SOUND OCCLUSION", "Off - walls do not muffle sound yet"),
        ],
        "SCREEN" => vec![
            Row::Toggle("VSYNC", Knob::Vsync),
            Row::Toggle("FULLSCREEN", Knob::Fullscreen),
        ],
        "GRAPHICS" => vec![
            Row::Number("FIELD OF VIEW", Knob::Fov, "vertical degrees"),
            Row::Fact(
                "RENDER DISTANCE",
                "160 m near ring, 2 km far mesh - fixed by the streaming budget",
            ),
            Row::Fact("ANTI-ALIASING", "SMAA, always on"),
            Row::Fact("AMBIENT OCCLUSION", "SSAO medium, always on"),
        ],
        "CONTROLS" => vec![
            Row::Number("MOUSE SENSITIVITY", Knob::Sensitivity, "x base"),
            Row::Toggle("INVERT LOOK", Knob::InvertLook),
        ],
        "KEYBINDS" => BINDS.iter().map(|(k, v)| Row::Fact(k, v)).collect(),
        _ => vec![Row::Note("Nothing here yet.")],
    }
}

/// The binds, read off `input::gather` and `pause::keys`. Read-only: rebinding
/// needs a stored map and a conflict check, which is its own slice. Drawn
/// anyway, because **a bind the player is never told about is a bind that does
/// not exist** — the rule the intro screen's numbered rows already obey.
pub const BINDS: [(&str, &str); 8] = [
    ("MOVE", "W A S D"),
    ("SPRINT", "Left Shift"),
    ("JUMP", "Space"),
    ("USE / ATTACK", "Left Mouse"),
    ("HOTBAR", "1 - 6"),
    ("LOOK", "Mouse (click to capture the pointer)"),
    ("MENU", "Esc"),
    ("QUIT", "Esc from the server list"),
];
/// Marks everything this screen owns.
#[derive(Component)]
pub struct SettingsRoot;

/// Marks the camera this screen had to spawn for itself, so a rebuild leaves
/// it alone. Respawning a camera on every click drops the frame it was
/// rendering.
#[derive(Component)]
pub struct SettingsCamera;

/// A rail row, by index into `CATEGORIES`.
#[derive(Component)]
pub struct Category(usize);

/// A control. `delta` is what a click applies: 0 for a toggle, +1/-1 for a
/// stepper.
#[derive(Component)]
pub struct Adjust {
    pub knob: Knob,
    pub delta: i32,
}

pub fn setup(
    mut commands: Commands,
    settings: Res<Settings>,
    disk: Option<Res<Disk>>,
    cameras: Query<(), With<Camera>>,
) {
    // Entered from the Esc menu there is a `Camera3d` up and the UI draws
    // against it; entered from the intro screen the menu's own camera went
    // with the menu. Two cameras would fight for the frame and Bevy would
    // warn, so this spawns one only when there is none.
    if cameras.is_empty() {
        commands.spawn((SettingsRoot, SettingsCamera, Camera2d));
    }
    build(&mut commands, &settings, disk.as_deref());
}

/// Rebuild after a click. Both the rail's selection and every drawn value are
/// derived from `Settings`, so the screen is respawned from it rather than
/// patched — a dozen nodes are cheaper to rebuild than to diff, and a patch
/// path is where a drawn value drifts from the held one.
pub fn rebuild(
    mut commands: Commands,
    mut settings: ResMut<Settings>,
    disk: Option<Res<Disk>>,
    roots: Query<Entity, (With<SettingsRoot>, Without<SettingsCamera>)>,
) {
    if !settings.dirty {
        return;
    }
    settings.dirty = false;
    for e in roots.iter() {
        commands.entity(e).despawn();
    }
    build(&mut commands, &settings, disk.as_deref());
}

/// The screen, as a plain function so `setup` and `rebuild` are provably the
/// same drawing — the shape `menu::build` already uses for the same reason.
fn build(commands: &mut Commands, settings: &Settings, disk: Option<&Disk>) {
    let back = match settings.back {
        Screen::Paused => "Esc goes back to the game",
        _ => "Esc goes back to the server list",
    };
    // The footer names the file, or says that nothing is being kept — the
    // honest states are "saved to <path>" and "not saved this run" (a
    // capture run, or a box whose environment names no config dir), and the
    // screen must not claim the wrong one.
    let saved = match disk {
        Some(d) => format!("saved to {}", d.path.display()),
        None => "settings are not being saved this run".to_string(),
    };

    commands
        .spawn((
            SettingsRoot,
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                ..default()
            },
            BackgroundColor(ui::BG),
        ))
        .with_children(|root| {
            root.spawn((
                ui::strong("SETTINGS", 30.0, ui::TITLE),
                Node {
                    margin: UiRect::new(Val::Px(34.0), Val::Px(0.0), Val::Px(26.0), Val::Px(16.0)),
                    ..default()
                },
            ));

            // The rail and the pane, side by side, both full height.
            root.spawn((Node {
                flex_direction: FlexDirection::Row,
                flex_grow: 1.0,
                column_gap: Val::Px(14.0),
                padding: UiRect::new(Val::Px(34.0), Val::Px(34.0), Val::Px(0.0), Val::Px(12.0)),
                ..default()
            },))
                .with_children(|body| {
                    // ---- the category rail -----------------------------------
                    body.spawn((
                        Node {
                            width: Val::Px(250.0),
                            flex_direction: FlexDirection::Column,
                            row_gap: Val::Px(2.0),
                            padding: UiRect::all(Val::Px(8.0)),
                            ..default()
                        },
                        BackgroundColor(ui::PANEL),
                    ))
                    .with_children(|rail| {
                        for (i, name) in CATEGORIES.iter().enumerate() {
                            let on = i == settings.cat;
                            let (idle, over) = if on {
                                (ui::ACCENT, ui::ACCENT_HOVER)
                            } else {
                                (ui::ROW_IDLE, ui::ROW_HOVER)
                            };
                            rail.spawn((
                                Button,
                                Category(i),
                                ui::Hover::new(idle, over),
                                Node {
                                    width: Val::Percent(100.0),
                                    padding: UiRect::axes(Val::Px(14.0), Val::Px(11.0)),
                                    border: UiRect::all(Val::Px(1.0)),
                                    ..default()
                                },
                                BackgroundColor(idle),
                                BorderColor::all(if on { ui::ACCENT } else { ui::RULE }),
                                children![ui::strong(
                                    *name,
                                    16.0,
                                    if on { ui::TEXT } else { ui::DIM }
                                )],
                            ));
                        }
                    });

                    // ---- the pane --------------------------------------------
                    body.spawn((
                        Node {
                            flex_grow: 1.0,
                            flex_direction: FlexDirection::Column,
                            row_gap: Val::Px(3.0),
                            padding: UiRect::all(Val::Px(14.0)),
                            overflow: Overflow::clip(),
                            ..default()
                        },
                        BackgroundColor(ui::PANEL),
                    ))
                    .with_children(|pane| {
                        for row in rows(settings.cat) {
                            spawn_row(pane, &row, settings);
                        }
                    });
                });

            root.spawn((
                ui::label(format!("{back}    -    {saved}"), 12.0, ui::FAINT),
                Node {
                    margin: UiRect::new(Val::Px(34.0), Val::Px(0.0), Val::Px(0.0), Val::Px(16.0)),
                    ..default()
                },
            ));
        });
}

/// One pane row. A function that SPAWNS rather than one that returns a bundle:
/// the four kinds are four different bundle types, and a match cannot return
/// four types from one `impl Bundle`.
fn spawn_row(
    pane: &mut bevy::ecs::relationship::RelatedSpawnerCommands<'_, ChildOf>,
    row: &Row,
    settings: &Settings,
) {
    // The label/value frame every row but a note shares.
    let frame = || Node {
        padding: UiRect::axes(Val::Px(12.0), Val::Px(8.0)),
        justify_content: JustifyContent::SpaceBetween,
        align_items: AlignItems::Center,
        column_gap: Val::Px(20.0),
        ..default()
    };

    match row {
        Row::Note(text) => {
            pane.spawn((
                Node {
                    padding: UiRect::axes(Val::Px(8.0), Val::Px(10.0)),
                    max_width: Val::Px(760.0),
                    ..default()
                },
                children![ui::label(*text, 14.0, ui::DIM)],
            ));
        }
        Row::Fact(label, value) => {
            pane.spawn((
                frame(),
                BackgroundColor(ui::ROW_IDLE),
                children![
                    ui::strong(*label, 15.0, ui::TEXT),
                    ui::label(*value, 13.0, ui::FAINT),
                ],
            ));
        }
        Row::Toggle(label, knob) => {
            pane.spawn((
                frame(),
                BackgroundColor(ui::ROW_IDLE),
                children![
                    ui::strong(*label, 15.0, ui::TEXT),
                    (
                        Button,
                        Adjust {
                            knob: *knob,
                            delta: 0,
                        },
                        ui::Hover::default(),
                        Node {
                            width: Val::Px(92.0),
                            height: Val::Px(26.0),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            border: UiRect::all(Val::Px(1.0)),
                            ..default()
                        },
                        BackgroundColor(ui::ROW_IDLE),
                        BorderColor::all(ui::RULE),
                        children![ui::strong(settings.value(*knob), 14.0, ui::TEXT)],
                    ),
                ],
            ));
        }
        Row::Number(label, knob, unit) => {
            pane.spawn((
                frame(),
                BackgroundColor(ui::ROW_IDLE),
                children![
                    ui::strong(*label, 15.0, ui::TEXT),
                    (
                        Node {
                            flex_direction: FlexDirection::Row,
                            align_items: AlignItems::Center,
                            column_gap: Val::Px(8.0),
                            ..default()
                        },
                        children![
                            ui::label(*unit, 12.0, ui::FAINT),
                            (
                                ui::stepper(),
                                Adjust {
                                    knob: *knob,
                                    delta: -1,
                                },
                                children![ui::strong("-", 16.0, ui::TEXT)],
                            ),
                            (
                                Node {
                                    width: Val::Px(52.0),
                                    justify_content: JustifyContent::Center,
                                    ..default()
                                },
                                children![ui::strong(settings.value(*knob), 15.0, ui::TEXT)],
                            ),
                            (
                                ui::stepper(),
                                Adjust {
                                    knob: *knob,
                                    delta: 1,
                                },
                                children![ui::strong("+", 16.0, ui::TEXT)],
                            ),
                        ],
                    ),
                ],
            ));
        }
    }
}

pub fn click(
    cats: Query<(&Interaction, &Category), Changed<Interaction>>,
    controls: Query<(&Interaction, &Adjust), Changed<Interaction>>,
    mut settings: ResMut<Settings>,
) {
    for (interaction, cat) in cats.iter() {
        if *interaction == Interaction::Pressed {
            settings.cat = cat.0;
            settings.dirty = true;
        }
    }
    for (interaction, adjust) in controls.iter() {
        if *interaction == Interaction::Pressed {
            settings.adjust(adjust.knob, adjust.delta);
        }
    }
}

pub fn keys(
    keyboard: Res<ButtonInput<KeyCode>>,
    settings: Res<Settings>,
    mut next: ResMut<NextState<Screen>>,
) {
    if keyboard.just_pressed(KeyCode::Escape) {
        next.set(settings.back.clone());
    }
}

pub fn teardown(mut commands: Commands, roots: Query<Entity, With<SettingsRoot>>) {
    for e in roots.iter() {
        commands.entity(e).despawn();
    }
}

/// Field of view onto the camera. Runs in every state: the camera may not
/// exist yet (menu) and may be rebuilt later (a second world), and a setting
/// that only applied while the settings screen was open would be a setting
/// that forgot itself on the way out.
pub fn apply_view(settings: Res<Settings>, mut cam: Query<&mut Projection, With<EyeCam>>) {
    let Ok(mut projection) = cam.single_mut() else {
        return;
    };
    if let Projection::Perspective(p) = &mut *projection {
        let want = settings.fov_deg.to_radians();
        if (p.fov - want).abs() > f32::EPSILON {
            p.fov = want;
        }
    }
}

/// Present mode and window mode. Same reasoning as `apply_view`, and the same
/// change guard: writing `Window` every frame marks it changed every frame,
/// which makes Bevy's window backend do work for nothing.
pub fn apply_window(settings: Res<Settings>, mut window: Query<&mut Window, With<PrimaryWindow>>) {
    let Ok(mut w) = window.single_mut() else {
        return;
    };
    let present = if settings.vsync {
        PresentMode::AutoVsync
    } else {
        PresentMode::AutoNoVsync
    };
    if w.present_mode != present {
        w.present_mode = present;
    }
    let mode = if settings.fullscreen {
        WindowMode::BorderlessFullscreen(MonitorSelection::Current)
    } else {
        WindowMode::Windowed
    };
    if w.mode != mode {
        w.mode = mode;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stepper_cannot_walk_a_setting_out_of_range() {
        let mut s = Settings::default();
        for _ in 0..100 {
            s.adjust(Knob::Fov, 1);
        }
        assert_eq!(s.fov_deg, FOV_MAX_DEG);
        for _ in 0..100 {
            s.adjust(Knob::Fov, -1);
        }
        assert_eq!(s.fov_deg, FOV_MIN_DEG);
    }

    #[test]
    fn sensitivity_lands_on_its_step_after_a_walk() {
        // The drift this guards: 40 float adds of 0.05 do not sum to 2.0, and
        // the value is PRINTED to two decimals, so the drift would be visible
        // before it was large.
        let mut s = Settings::default();
        for _ in 0..40 {
            s.adjust(Knob::Sensitivity, 1);
        }
        for _ in 0..40 {
            s.adjust(Knob::Sensitivity, -1);
        }
        assert_eq!(s.value(Knob::Sensitivity), "1.00");
        assert!((s.sensitivity - 1.0).abs() < 1e-6, "{}", s.sensitivity);
    }

    #[test]
    fn a_toggle_ignores_the_delta_and_flips() {
        let mut s = Settings::default();
        assert!(s.vsync);
        s.adjust(Knob::Vsync, 0);
        assert!(!s.vsync);
        assert_eq!(s.value(Knob::Vsync), "OFF");
    }

    #[test]
    fn every_category_draws_something() {
        // The failure this catches is a category rail row that opens an empty
        // pane — the "dark panel that cannot say what would light it" this
        // repo has a rule about. An empty category must carry a NOTE.
        for (i, name) in CATEGORIES.iter().enumerate() {
            assert!(!rows(i).is_empty(), "{name} draws nothing");
        }
    }

    #[test]
    fn the_defaults_are_the_rig_the_client_ships() {
        // A settings screen that opens on a value the renderer is not using
        // is a screen that lies on its first frame. FOV is the one setting
        // whose default is owned somewhere else.
        assert_eq!(Settings::default().fov_deg, FOV_DEG);
        assert_eq!(Settings::default().sensitivity, 1.0);
    }

    #[test]
    fn what_a_player_set_survives_a_round_trip() {
        // The whole slice in one assertion: walk every knob off its default,
        // serialize what would be written, parse it back over the defaults,
        // and the settings that come up are the settings that were set.
        let mut s = Settings::default();
        s.adjust(Knob::Fov, 2);
        s.adjust(Knob::Sensitivity, -3);
        s.adjust(Knob::InvertLook, 0);
        s.adjust(Knob::Vsync, 0);
        s.adjust(Knob::Fullscreen, 0);
        s.adjust(Knob::VolMaster, -2);
        s.adjust(Knob::VolGame, -5);
        s.adjust(Knob::VolAmbience, -10);
        let text = config::serialize(&s.persisted(), config::SETTINGS_VERSION, &[], &[]);
        let back =
            Settings::from_persisted(config::parse(&text, Settings::default().persisted()).values);
        assert_eq!(back.persisted(), s.persisted(), "{text}");
    }

    #[test]
    fn a_hand_edited_file_cannot_reach_what_a_hundred_clicks_cannot() {
        // `config::parse` already refused non-finite floats; this is the
        // range half, applied at load beside the steppers' own bounds. The
        // clamp is the same call `adjust` makes, so the two paths cannot
        // disagree about where the ends are.
        let wild = Persisted {
            fov_deg: 500.0,
            sensitivity: -2.0,
            vol_master: 7.0,
            vol_game: -0.4,
            // Off the step: the screen prints tenths, so the file loads tenths.
            vol_ambience: 0.44,
            ..Settings::default().persisted()
        };
        let s = Settings::from_persisted(wild);
        assert_eq!(s.fov_deg, FOV_MAX_DEG);
        assert_eq!(s.sensitivity, SENS_MIN);
        assert_eq!(s.vol_master, 1.0);
        assert_eq!(s.vol_game, 0.0);
        assert_eq!(s.value(Knob::VolAmbience), "40%");
    }

    #[test]
    fn loading_the_defaults_changes_nothing() {
        // A fresh boot must be byte-for-byte the old behaviour: no file is
        // the defaults, and the defaults sanitized are still the defaults —
        // a default off its own step would mean every boot "changed" a value
        // and the first click wrote a file the player never asked for.
        let d = Settings::default().persisted();
        assert_eq!(Settings::from_persisted(d).persisted(), d);
    }
}
