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
//!
//! **This file also owns the FACE**, for the same reason it owns the palette,
//! and until 2026-08-07 nothing did: all 42 `TextFont` sites in the render
//! path were `..default()`, so every screen this client has was drawn in
//! Bevy's embedded debug mono. That is not a style — it is the absence of
//! one, and it is the single loudest thing separating our frames from the
//! reference's.

use bevy::asset::{uuid_handle, AssetId};
use bevy::prelude::*;
use bevy::text::Font;

// ---- the face ------------------------------------------------------------

/// The two faces, at compile-time-known handles.
///
/// **Roboto Condensed, and that is measured rather than chosen.** The
/// reference game's own modding API names its default UI font in public
/// source: `Assets/Content/UI/Fonts/RobotoCondensed-Bold.ttf`, with
/// `RobotoCondensed-Regular` beside it and Bold as the fallback when a
/// lookup misses (`Facepunch/Rust.Community`, `CommunityEntity.UI.cs`
/// `LoadFont`). So the weight hierarchy below is theirs too: **bold is the
/// default and regular is the exception**, which is why a Rust screen reads
/// as chunky and ours read as a terminal.
///
/// Apache-2.0, © 2011 Google Inc. — `crates/client/fonts/`.
pub const SANS: Handle<Font> = uuid_handle!("6f0d1c8e-2f4a-4c7b-9a31-0d2b7c5e4a11");
pub const SANS_BOLD: Handle<Font> = uuid_handle!("6f0d1c8e-2f4a-4c7b-9a31-0d2b7c5e4a12");

/// **Embedded, not loaded from `assets/`, and both halves of that are
/// deliberate.**
///
/// A `Handle<Font>` that never resolves draws *nothing at all* — not a
/// fallback glyph, not a box. That is strictly worse than the failure this
/// repo has already been bitten by twice (a missing `jpeg` decoder made Bevy
/// draw a white texture *and keep going*, and three material changes measured
/// byte-identical before anyone read the log), because a client with no text
/// cannot report its own defect. Compiling the bytes in removes the failure
/// mode rather than gating it.
///
/// The second half is timing, and it is a trap this repo has paid for once:
/// **`OnEnter(Screen::Loading)` runs before `Startup`**, so the loading
/// screen — the first thing a connected start draws — would render its text
/// blank for as long as an asset load took. `audio::build_bank` is the same
/// shape for the same reason, and [`build_fonts`] is called beside it.
const SANS_TTF: &[u8] = include_bytes!("../../fonts/RobotoCondensed-Regular.ttf");
const SANS_BOLD_TTF: &[u8] = include_bytes!("../../fonts/RobotoCondensed-Bold.ttf");

/// Put both faces in `Assets<Font>` at plugin-build time.
///
/// It also overwrites **Bevy's own default font handle** with the bold face.
/// That is the belt to [`font`]'s braces: the gate in `tests/ui.rs` is what
/// keeps `TextFont` literals out of the render path, but a site that slips
/// past it should draw in the wrong *weight*, not in a different typeface
/// from every other word on the screen.
pub fn build_fonts(app: &mut App) {
    let mut assets = app.world_mut().resource_mut::<Assets<Font>>();
    for (id, bytes, what) in [
        (SANS.id(), SANS_TTF, "RobotoCondensed-Regular"),
        (SANS_BOLD.id(), SANS_BOLD_TTF, "RobotoCondensed-Bold"),
        (AssetId::default(), SANS_BOLD_TTF, "the engine default"),
    ] {
        let font = Font::try_from_bytes(bytes.to_vec())
            .unwrap_or_else(|e| panic!("{what} is not a readable font: {e}"));
        assets
            .insert(id, font)
            .unwrap_or_else(|e| panic!("{what} could not be registered: {e}"));
    }
}

/// A `TextFont` in the regular face. **Prose only** — a description, a hint,
/// a status line, a chat message.
pub fn font(size: f32) -> TextFont {
    TextFont {
        font: SANS,
        font_size: size,
        ..default()
    }
}

/// A `TextFont` in the bold face — the reference's default, and ours. Every
/// label, heading, number, button and item name.
pub fn font_bold(size: f32) -> TextFont {
    TextFont {
        font: SANS_BOLD,
        font_size: size,
        ..default()
    }
}

// ---- the palette, re-derived 2026-08-07 ----------------------------------
//
// **Every value below is sampled off `Rust Images/crafting.png`**, the
// reference's own inventory frame, by averaging a 7×7 patch at the named
// place. The previous palette was neither measured nor close: it was a
// **cool near-black** (`0.082,0.082,0.090` — blue the largest channel) with
// an **olive** selection, against a reference that is **warm** (R > G > B in
// every single sample) about **twice as light**, and selects in **blue**.
// That is a different family, not a near miss, and no amount of type work
// reads as the reference while the ground under it is the wrong colour.
//
// The olive it replaces was described as measured off "the reference's own
// options screen" — a frame that is not in `Rust Images/` and cannot be
// re-checked. If the options screen really does select in olive, that is a
// per-screen exception to add back with the frame committed beside it; it is
// not the inventory's colour, and the inventory is the screen a player lives
// in. `DECISIONS.md` "menu skin v0" carries the change.
//
// **One honest caveat**: the reference panel is translucent over a blurred
// world, so a sample carries some of whatever was behind it. These are the
// composited values — which is what a player sees, and still far closer than
// an invented number.

/// The screen behind everything. Darker than any panel, so a full-screen
/// menu still reads as a layer rather than as the desktop.
pub const BG: Color = Color::srgb(0.106, 0.098, 0.086);
/// A panel sitting on the background — the settings sidebar and pane.
/// `#423e39`, the reference's category rail.
pub const PANEL: Color = Color::srgba(0.259, 0.243, 0.224, 1.0);
/// A clickable row at rest, and under the pointer. `#2b2723` / lifted.
pub const ROW_IDLE: Color = Color::srgba(0.169, 0.153, 0.137, 1.0);
pub const ROW_HOVER: Color = Color::srgba(0.278, 0.263, 0.235, 1.0);
/// The selected category — `#3982ba`, and the one saturated block on the
/// reference's inventory screen. Everything else there is warm grey, which
/// is exactly why the selection is a cool blue: it is the only hue on the
/// panel and it never has to compete.
pub const ACCENT: Color = Color::srgb(0.224, 0.510, 0.729);
pub const ACCENT_HOVER: Color = Color::srgb(0.290, 0.580, 0.800);
/// Hairline borders.
pub const RULE: Color = Color::srgba(0.75, 0.72, 0.62, 0.28);
/// The shell's two grounds, and they are the same family as `BG` rather than
/// new hues: the reference's front end is a scrim over a rendered scene, so
/// its panels are *darker* than the world behind them and read as depth. Ours
/// has no scene behind it yet (`NOW.md` — the menu backdrop is the follow-on
/// this shell makes possible), so the two grounds carry the depth on their
/// own: the control column sits above the page, the content pane below it.
pub const PANEL_BG: Color = Color::srgba(0.145, 0.133, 0.118, 1.0);
pub const PANE_BG: Color = Color::srgba(0.086, 0.078, 0.070, 1.0);
/// The wordmark's block. The one saturated thing on the front screen, and the
/// reference makes the same call — its logo mark is the only colour on an
/// otherwise grey menu.
pub const MARK: Color = Color::srgb(0.804, 0.243, 0.176);
/// A nav entry at rest. Dimmer than `DIM`, because the whole effect depends
/// on the picked entry being obviously brighter than the rest.
pub const NAV_IDLE: Color = Color::srgba(0.62, 0.60, 0.57, 0.65);
/// Type, in three weights of attention.
pub const TITLE: Color = Color::srgb(0.93, 0.92, 0.90);
pub const TEXT: Color = Color::srgb(0.92, 0.90, 0.85);
pub const DIM: Color = Color::srgba(0.74, 0.72, 0.68, 0.85);
pub const FAINT: Color = Color::srgba(0.62, 0.60, 0.56, 0.80);

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

/// A screen's own heading — `YOU DIED`, `DISCONNECTED`.
///
/// **Not the wordmark**, which is [`wordmark`] and is the game's identity
/// rather than a screen's. The two were the same call until the shell landed
/// and every screen's heading was the word GATES; they are different jobs, and
/// a screen that shouts its own name where the reference shouts what just
/// happened to you is a screen that has nothing to say.
pub fn title(text: &str) -> impl Bundle {
    (
        Text::new(text.to_string()),
        font_bold(58.0),
        TextColor(TITLE),
    )
}

/// A line of prose, in the regular face. The three colours above are the
/// whole vocabulary.
///
/// **There are twelve distinct sizes across the six files that draw text**
/// (58, 30, 26, 20, 17, 16, 15, 14, 13, 12, 11, 10) and that is not a
/// hierarchy, it is what twelve independent decisions look like. Collapsing
/// it to five steps is a real improvement and it is deliberately **not**
/// taken here: the sizes were budgeted against 720p (`DECISIONS.md`, "menu
/// skin v0" — the first cut clipped a column at both ends) and nothing in
/// this repo can photograph a panel, so moving them would be tuning a number
/// with the picture unavailable. `NOW.md` carries it behind the screenshot
/// tool that would make it checkable.
pub fn label(text: impl Into<String>, size: f32, color: Color) -> impl Bundle {
    (Text::new(text.into()), font(size), TextColor(color))
}

/// A line of type in the bold face — a label, a heading, a number, a verb.
/// The reference's default weight, and therefore the common case.
pub fn strong(text: impl Into<String>, size: f32, color: Color) -> impl Bundle {
    (Text::new(text.into()), font_bold(size), TextColor(color))
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

// ---- the shell ------------------------------------------------------------
//
// **The five reference frames the operator pasted are one screen, not five.**
// Main menu, PLAY GAME, NEWS, INVENTORY and WORKSHOP all draw: a wordmark
// pinned top-left, a vertical nav column under it that never moves and never
// re-orders, and — for every entry except the bare menu — a tinted control
// panel plus a wide content pane filling the rest. Picking a nav entry swaps
// what is in those two columns and touches nothing else. That single fact is
// why their front end reads as one product and why ours read as five
// unrelated screens: we had five independent full-screen states, each
// centring its own column, each rebuilding its own title.
//
// So the shell is the thing worth copying, ahead of any individual screen.
// What is deliberately NOT copied is their nav's length — NEWS, INVENTORY,
// ITEM STORE, WORKSHOP and RUST+ are five entries with a live service behind
// each, and drawing a dead one is the greyed-row dishonesty `settings.rs`
// already refuses.

/// The wordmark, top-left, at the size the reference draws its own.
///
/// A text mark rather than an image: this repo has no logo asset and an
/// invented one is a brand decision, not a render one. The block that
/// precedes it is the reference's own arrangement — a mark, then the word —
/// and it is the one piece of visual identity the shell needs to stop looking
/// like a debug overlay.
pub fn wordmark() -> impl Bundle {
    (
        Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: Val::Px(12.0),
            ..default()
        },
        children![
            (
                Node {
                    width: Val::Px(38.0),
                    height: Val::Px(38.0),
                    ..default()
                },
                BackgroundColor(MARK),
            ),
            strong("GATES", 40.0, TITLE),
        ],
    )
}

/// One entry in the nav column.
///
/// **Dim at rest, bright when picked** — the reference's exact treatment, and
/// the reason its menu has no buttons on it: the type *is* the button, so the
/// screen carries no chrome it does not need. `Hover` handles the pointer,
/// and the colour a nav entry rests at is carried per entity because the
/// picked one must not be repainted on the way past it (the same reason
/// [`Hover`] exists at all).
pub fn nav_item(text: &str, picked: bool) -> impl Bundle {
    (
        Button,
        Node {
            padding: UiRect::axes(Val::Px(0.0), Val::Px(7.0)),
            ..default()
        },
        // Transparent, not a panel: the nav floats over the backdrop.
        BackgroundColor(Color::NONE),
        Hover::new(Color::NONE, Color::NONE),
        children![strong(
            text.to_string(),
            26.0,
            if picked { TITLE } else { NAV_IDLE },
        )],
    )
}

/// The tinted control column — the reference's filter panel, its YOUR ITEMS
/// column, its resource header.
///
/// **Its tint is ours, and that is stated rather than hidden.** The
/// reference's is a warm red at low alpha over the blurred scene; the frames
/// that show it are not in `Rust Images/`, so it cannot be sampled the way
/// every colour in the palette above was, and inventing a number while
/// claiming a measurement is the failure mode `ART.md` exists to prevent.
/// This is [`PANEL`] — measured, off `crafting.png` — until a frame lands
/// beside it to re-derive from.
pub fn panel(width_px: f32) -> impl Bundle {
    (
        Node {
            width: Val::Px(width_px),
            height: Val::Percent(100.0),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(10.0),
            padding: UiRect::all(Val::Px(16.0)),
            ..default()
        },
        BackgroundColor(PANEL_BG),
    )
}

/// The wide content column beside it.
///
/// The right padding is not cosmetic slack: without it a right-aligned column
/// — the server list's PING — sits flush against the window edge, which reads
/// as a table that has been cut off rather than one that ends. The reference
/// leaves the same margin.
pub fn pane() -> impl Bundle {
    (
        Node {
            flex_grow: 1.0,
            height: Val::Percent(100.0),
            flex_direction: FlexDirection::Column,
            padding: UiRect::new(Val::Px(0.0), Val::Px(20.0), Val::Px(0.0), Val::Px(0.0)),
            ..default()
        },
        BackgroundColor(PANE_BG),
    )
}

/// A small heading over a group of controls — the reference's `PLAYER SLOTS`
/// and `WIPE SCHEDULE`.
pub fn group(text: &str) -> impl Bundle {
    (
        strong(text.to_string(), 12.0, FAINT),
        Node {
            margin: UiRect::top(Val::Px(6.0)),
            ..default()
        },
    )
}

/// A wide flat button — CLEAR FILTERS, REFRESH, Create a new item.
pub fn button(width_px: f32, accent: bool) -> impl Bundle {
    (
        Button,
        Node {
            width: Val::Px(width_px),
            padding: UiRect::axes(Val::Px(10.0), Val::Px(9.0)),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        },
        BackgroundColor(if accent { ACCENT } else { ROW_IDLE }),
        Hover::new(
            if accent { ACCENT } else { ROW_IDLE },
            if accent { ACCENT_HOVER } else { ROW_HOVER },
        ),
    )
}

/// A checkable filter row — the reference's radio group drawn as a mark plus
/// a label, because a filter that is a toggle should not draw a radio.
pub fn check(on: bool) -> impl Bundle {
    (
        Button,
        Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: Val::Px(9.0),
            padding: UiRect::axes(Val::Px(2.0), Val::Px(5.0)),
            ..default()
        },
        BackgroundColor(Color::NONE),
        Hover::new(Color::NONE, ROW_IDLE),
        children![(
            Node {
                width: Val::Px(13.0),
                height: Val::Px(13.0),
                border: UiRect::all(Val::Px(1.0)),
                ..default()
            },
            BackgroundColor(if on { ACCENT } else { Color::NONE }),
            BorderColor::all(RULE),
        )],
    )
}

/// A table row in the content pane — the server list's, and whatever comes
/// after it. Full width, hairline-separated, hovering as one object.
pub fn table_row() -> impl Bundle {
    (
        Button,
        Node {
            width: Val::Percent(100.0),
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: Val::Px(10.0),
            padding: UiRect::axes(Val::Px(12.0), Val::Px(9.0)),
            border: UiRect::bottom(Val::Px(1.0)),
            ..default()
        },
        BackgroundColor(ROW_IDLE),
        BorderColor::all(RULE),
        Hover::default(),
    )
}

/// A fixed-width right-aligned column — `PLAYERS` and `PING`, header and cell
/// alike, so the two cannot drift apart by being sized independently.
pub fn column(width_px: f32) -> Node {
    Node {
        width: Val::Px(width_px),
        flex_direction: FlexDirection::Column,
        align_items: AlignItems::FlexEnd,
        ..default()
    }
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
