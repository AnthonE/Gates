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
//! **Nothing here is persisted yet.** Settings live for the run and go back to
//! their defaults on the next launch; a config file is its own slice (a path,
//! a format, and a version for when a knob is renamed). Said in the footer, so
//! the screen does not quietly forget on the player's behalf.
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
        }
    }
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
        "AUDIO" => vec![Row::Note(
            "This client renders no audio at all. When it does, its mixer's knobs \
             belong here.",
        )],
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

pub fn setup(mut commands: Commands, settings: Res<Settings>, cameras: Query<(), With<Camera>>) {
    // Entered from the Esc menu there is a `Camera3d` up and the UI draws
    // against it; entered from the intro screen the menu's own camera went
    // with the menu. Two cameras would fight for the frame and Bevy would
    // warn, so this spawns one only when there is none.
    if cameras.is_empty() {
        commands.spawn((SettingsRoot, SettingsCamera, Camera2d));
    }
    build(&mut commands, &settings);
}

/// Rebuild after a click. Both the rail's selection and every drawn value are
/// derived from `Settings`, so the screen is respawned from it rather than
/// patched — a dozen nodes are cheaper to rebuild than to diff, and a patch
/// path is where a drawn value drifts from the held one.
pub fn rebuild(
    mut commands: Commands,
    mut settings: ResMut<Settings>,
    roots: Query<Entity, (With<SettingsRoot>, Without<SettingsCamera>)>,
) {
    if !settings.dirty {
        return;
    }
    settings.dirty = false;
    for e in roots.iter() {
        commands.entity(e).despawn();
    }
    build(&mut commands, &settings);
}

/// The screen, as a plain function so `setup` and `rebuild` are provably the
/// same drawing — the shape `menu::build` already uses for the same reason.
fn build(commands: &mut Commands, settings: &Settings) {
    let back = match settings.back {
        Screen::Paused => "Esc goes back to the game",
        _ => "Esc goes back to the server list",
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
                ui::label("SETTINGS", 30.0, ui::TITLE),
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
                                children![ui::label(
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
                ui::label(
                    format!("{back}    -    nothing here is saved between runs yet"),
                    12.0,
                    ui::FAINT,
                ),
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
                    ui::label(*label, 15.0, ui::TEXT),
                    ui::label(*value, 13.0, ui::FAINT),
                ],
            ));
        }
        Row::Toggle(label, knob) => {
            pane.spawn((
                frame(),
                BackgroundColor(ui::ROW_IDLE),
                children![
                    ui::label(*label, 15.0, ui::TEXT),
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
                        children![ui::label(settings.value(*knob), 14.0, ui::TEXT)],
                    ),
                ],
            ));
        }
        Row::Number(label, knob, unit) => {
            pane.spawn((
                frame(),
                BackgroundColor(ui::ROW_IDLE),
                children![
                    ui::label(*label, 15.0, ui::TEXT),
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
                                children![ui::label("-", 16.0, ui::TEXT)],
                            ),
                            (
                                Node {
                                    width: Val::Px(52.0),
                                    justify_content: JustifyContent::Center,
                                    ..default()
                                },
                                children![ui::label(settings.value(*knob), 15.0, ui::TEXT)],
                            ),
                            (
                                ui::stepper(),
                                Adjust {
                                    knob: *knob,
                                    delta: 1,
                                },
                                children![ui::label("+", 16.0, ui::TEXT)],
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
}
