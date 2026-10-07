//! The panel kit (`ARC.md` F4): the pieces every in-world screen is made of
//! — the card over the scrim, its header, a section rule, a progress bar, a
//! button, a key hint — so a new screen is composition and nobody hand-rolls
//! a card again.
//!
//! The older panels (inventory, crafting, tree, kiosk) predate it and still
//! build their own nodes; they move onto it as they are next touched. Colours
//! and faces are the panel palette's (`super`), so a kit screen and an old
//! one read as one product.

use bevy::prelude::*;

use super::{
    font, font_bold, Hover, PanelRoot, BADGE, CELL_BG, CELL_FULL, LINE, LINE_HOT, PANEL_BG, SCRIM,
    TEXT, TEXT_DIM, TEXT_SHORT, VITAL_TROUGH,
};

/// The fill of a bar that is filling (a quota, a share).
pub const BAR_FILL: Color = Color::srgb(0.84, 0.62, 0.27);
/// The fill of a tank — fuel, burning.
pub const BAR_TANK: Color = Color::srgb(0.88, 0.40, 0.18);

/// A full-screen panel: the scrim, and a card `width` px wide centred on it,
/// with `body` building the card's contents.
pub fn screen(commands: &mut Commands, width: f32, body: impl FnOnce(&mut ChildSpawnerCommands)) {
    commands
        .spawn((
            PanelRoot,
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            },
            BackgroundColor(SCRIM),
        ))
        .with_children(|root| {
            root.spawn((
                Node {
                    width: Val::Px(width),
                    max_height: Val::Percent(88.0),
                    flex_direction: FlexDirection::Column,
                    padding: UiRect::all(Val::Px(14.0)),
                    row_gap: Val::Px(6.0),
                    border: UiRect::all(Val::Px(1.0)),
                    overflow: Overflow::clip_y(),
                    ..default()
                },
                BackgroundColor(PANEL_BG),
                BorderColor::all(LINE),
            ))
            .with_children(body);
        });
}

/// The card's head: a title on the left, an optional right-hand label, and
/// the status line under them — the one line that says what just happened.
pub fn header(p: &mut ChildSpawnerCommands, title: &str, right: Option<&str>, status: &str) {
    p.spawn(Node {
        flex_direction: FlexDirection::Row,
        justify_content: JustifyContent::SpaceBetween,
        align_items: AlignItems::Center,
        ..default()
    })
    .with_children(|h| {
        h.spawn((Text::new(title), font_bold(18.0), TextColor(BADGE)));
        if let Some(r) = right {
            h.spawn((Text::new(r), font_bold(13.0), TextColor(TEXT_DIM)));
        }
    });
    if !status.is_empty() {
        p.spawn((Text::new(status), font(12.0), TextColor(LINE_HOT)));
    }
}

/// A line of text.
pub fn line(p: &mut ChildSpawnerCommands, text: impl Into<String>, size: f32, color: Color) {
    p.spawn((Text::new(text.into()), font(size), TextColor(color)));
}

/// A bold line of text.
pub fn strong(p: &mut ChildSpawnerCommands, text: impl Into<String>, size: f32, color: Color) {
    p.spawn((Text::new(text.into()), font_bold(size), TextColor(color)));
}

/// A section rule: a small caps title over a hairline.
pub fn section(p: &mut ChildSpawnerCommands, title: &str) {
    p.spawn((
        Node {
            margin: UiRect::top(Val::Px(6.0)),
            padding: UiRect::bottom(Val::Px(2.0)),
            border: UiRect::bottom(Val::Px(1.0)),
            ..default()
        },
        BorderColor::all(LINE),
    ))
    .with_children(|s| {
        s.spawn((Text::new(title), font_bold(11.0), TextColor(TEXT_DIM)));
    });
}

/// A progress bar, `pm` per mille full, the card's width unless `width` says.
pub fn bar(p: &mut ChildSpawnerCommands, pm: u32, fill: Color, width: Option<f32>) {
    p.spawn((
        Node {
            width: width.map_or(Val::Percent(100.0), Val::Px),
            height: Val::Px(8.0),
            ..default()
        },
        BackgroundColor(VITAL_TROUGH),
    ))
    .with_children(|t| {
        t.spawn((
            Node {
                width: Val::Percent(pm.min(1000) as f32 / 10.0),
                height: Val::Percent(100.0),
                ..default()
            },
            BackgroundColor(fill),
        ));
    });
}

/// A button carrying `marker`. `armed` draws it live; otherwise it is the
/// dim one (still clickable, so the sim can say why).
pub fn button<M: Component>(p: &mut ChildSpawnerCommands, label: &str, marker: M, armed: bool) {
    let bg = if armed { CELL_FULL } else { CELL_BG };
    p.spawn((
        Button,
        marker,
        Node {
            padding: UiRect::axes(Val::Px(10.0), Val::Px(4.0)),
            border: UiRect::all(Val::Px(1.0)),
            ..default()
        },
        BackgroundColor(bg),
        Hover::on(bg),
        BorderColor::all(if armed { LINE_HOT } else { LINE }),
    ))
    .with_children(|b| {
        b.spawn((
            Text::new(label),
            font_bold(12.0),
            TextColor(if armed { LINE_HOT } else { TEXT_DIM }),
            Pickable::IGNORE,
        ));
    });
}

/// A row: children laid out left to right, centred, with a gap.
pub fn row(p: &mut ChildSpawnerCommands, body: impl FnOnce(&mut ChildSpawnerCommands)) {
    p.spawn(Node {
        flex_direction: FlexDirection::Row,
        align_items: AlignItems::Center,
        column_gap: Val::Px(10.0),
        ..default()
    })
    .with_children(body);
}

/// A fixed-width cell of text inside a [`row`].
pub fn cell(p: &mut ChildSpawnerCommands, text: impl Into<String>, width: f32, color: Color) {
    p.spawn((
        Node {
            width: Val::Px(width),
            ..default()
        },
        Pickable::IGNORE,
    ))
    .with_children(|c| {
        c.spawn((Text::new(text.into()), font(13.0), TextColor(color)));
    });
}

/// The key hint at the foot of a card.
pub fn hint(p: &mut ChildSpawnerCommands, text: &str) {
    p.spawn((
        Node {
            margin: UiRect::top(Val::Px(4.0)),
            ..default()
        },
        Pickable::IGNORE,
    ))
    .with_children(|h| {
        h.spawn((Text::new(text), font(11.0), TextColor(TEXT_DIM)));
    });
}

/// The colour a count is drawn in: short (red) when it falls under `need`.
pub fn count_color(have: u32, need: u32) -> Color {
    if have >= need {
        TEXT
    } else {
        TEXT_SHORT
    }
}
