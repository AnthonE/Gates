//! The chrome the four menu screens share: the intro, the loading screen, the
//! Esc menu and settings.
//!
//! **Why this exists at all.** The Esc menu is the intro screen seen from
//! inside the world — same title, same row list, same way out — and settings
//! is reachable from both. Three screens that are supposed to read as one
//! product had, on the first cut, three copies of the same six colours and
//! two copies of the same hover handler. A palette in one file is the cheapest
//! form of "one product", and a hover that lives here cannot disagree with
//! itself between screens.
//!
//! **Bundles, not spawn helpers.** Every constructor below returns
//! `impl Bundle` rather than taking a child-spawner: a bundle composes with
//! `children![]` and with any marker the caller wants to add, and it does not
//! pin this module to the name of Bevy's spawner type, which has been renamed
//! twice in as many releases.
//!
//! Colours are read off the reference's own options screen (`Rust Images/`,
//! and the frame the operator pasted): near-black panels a shade lighter than
//! the background, an olive selection block for the chosen category, warm
//! off-white type, and a hairline rule the same warm hue at low alpha.

use bevy::prelude::*;

/// The screen behind everything.
pub const BG: Color = Color::srgb(0.055, 0.055, 0.060);
/// A panel sitting on the background — the settings sidebar and pane.
pub const PANEL: Color = Color::srgba(0.082, 0.082, 0.090, 1.0);
/// A clickable row at rest, and under the pointer.
pub const ROW_IDLE: Color = Color::srgba(0.10, 0.10, 0.11, 1.0);
pub const ROW_HOVER: Color = Color::srgba(0.16, 0.15, 0.13, 1.0);
/// The selected category. The reference's olive, which is the one saturated
/// block on its whole options screen — everything else is grey.
pub const ACCENT: Color = Color::srgb(0.33, 0.45, 0.17);
pub const ACCENT_HOVER: Color = Color::srgb(0.38, 0.51, 0.20);
/// Hairline borders.
pub const RULE: Color = Color::srgba(0.75, 0.72, 0.62, 0.28);
/// Type, in three weights of attention.
pub const TITLE: Color = Color::srgb(0.86, 0.83, 0.76);
pub const TEXT: Color = Color::srgb(0.92, 0.90, 0.85);
pub const DIM: Color = Color::srgba(0.70, 0.68, 0.62, 0.80);
pub const FAINT: Color = Color::srgba(0.60, 0.58, 0.54, 0.75);

/// What a button's background is at rest and under the pointer.
///
/// Carried per entity rather than assumed, because the settings sidebar's
/// selected row is olive and every other button is grey, and a hover system
/// that hard-coded one pair would repaint the selection on the way past it.
#[derive(Component)]
pub struct Hover {
    pub idle: Color,
    pub over: Color,
}

impl Default for Hover {
    fn default() -> Self {
        Self {
            idle: ROW_IDLE,
            over: ROW_HOVER,
        }
    }
}

impl Hover {
    pub fn new(idle: Color, over: Color) -> Self {
        Self { idle, over }
    }
}

/// One hover handler for every screen. Runs wherever a menu is up; a button
/// with no `Hover` is simply not repainted, which is what an inert row wants.
pub fn hover(mut q: Query<(&Interaction, &Hover, &mut BackgroundColor), Changed<Interaction>>) {
    for (interaction, h, mut bg) in q.iter_mut() {
        *bg = BackgroundColor(match interaction {
            Interaction::None => h.idle,
            _ => h.over,
        });
    }
}

/// A full-screen column: the root every menu screen hangs off.
///
/// Absolute and 100% on both axes so it covers whatever is behind it — which
/// in the loading screen and the Esc menu is the world, still rendering.
pub fn screen(bg: Color) -> impl Bundle {
    (
        Node {
            position_type: PositionType::Absolute,
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            flex_direction: FlexDirection::Column,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            row_gap: Val::Px(10.0),
            ..default()
        },
        BackgroundColor(bg),
    )
}

/// GATES, at the top of whichever screen the player is on.
pub fn title(text: &str) -> impl Bundle {
    (
        Text::new(text.to_string()),
        TextFont {
            font_size: 58.0,
            ..default()
        },
        TextColor(TITLE),
    )
}

/// A line of type. The three colours above are the whole vocabulary.
pub fn label(text: impl Into<String>, size: f32, color: Color) -> impl Bundle {
    (
        Text::new(text.into()),
        TextFont {
            font_size: size,
            ..default()
        },
        TextColor(color),
    )
}

/// A wide clickable row — the intro screen's shard rows and the Esc menu's
/// verbs are the same object at the same width.
pub fn row(width_px: f32) -> impl Bundle {
    (
        Button,
        Node {
            width: Val::Px(width_px),
            padding: UiRect::axes(Val::Px(16.0), Val::Px(10.0)),
            border: UiRect::all(Val::Px(1.0)),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(3.0),
            ..default()
        },
        BackgroundColor(ROW_IDLE),
        BorderColor::all(RULE),
        Hover::default(),
    )
}

/// A small square button — the `-` and `+` either side of a numeric setting.
pub fn stepper() -> impl Bundle {
    (
        Button,
        Node {
            width: Val::Px(30.0),
            height: Val::Px(26.0),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            border: UiRect::all(Val::Px(1.0)),
            ..default()
        },
        BackgroundColor(ROW_IDLE),
        BorderColor::all(RULE),
        Hover::default(),
    )
}
