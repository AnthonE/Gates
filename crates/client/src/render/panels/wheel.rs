//! The radial build menu.
//!
//! Held open with `B`, released to keep whatever it was last over. The
//! geometry is [`crate::ui::build`]'s and the picking is
//! [`crate::ui::build::pick`] — **not** Bevy's node hit-testing, and that is
//! the design rather than an accident:
//!
//! - a wedge is not a rectangle, so node picking would need one collider per
//!   segment and would still be wrong at the corners;
//! - the angle arithmetic is the part that can be subtly wrong (off by half
//!   a segment, or clockwise where the labels are anticlockwise), and it is
//!   only testable if a test can call it without a window;
//! - the labels then become pure decoration, and decoration cannot disagree
//!   with the selection, because it is not what is asked.
//!
//! So the ring nodes are drawn from `segment_angle` and the pointer is
//! resolved by `pick`, and `crates/client/tests/ui.rs` drives both.
//!
//! ## What choosing does, and what it does not do yet
//!
//! It latches the piece. It does **not** place one: the native client has no
//! build ghost — nothing draws the cell you are aiming at, nothing colours
//! it by whether the sim would accept it — and placing a piece the player
//! cannot see the destination of spends materials on a guess. Selection and
//! placement are two slices and this is the first; `NOW.md` carries the
//! second, which is where `encode_action_place`'s cell, level and location
//! arguments get their aiming.
//!
//! The latch is visible: the HUD draws the chosen piece, so the wheel's
//! effect is on screen rather than in a field nothing reads.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use client_wasm::core::ClientCore;

use super::{
    font, font_bold, Panel, Ui, BADGE, CELL_BG, CELL_FULL, LINE, LINE_HOT, PANEL_BG, TEXT,
    TEXT_DIM, TEXT_SHORT,
};
use crate::ui::build::{
    costs, material_label, row_for, segment_angle, shape_blurb, shape_label, Hover, Rings,
    MATERIALS, SHAPES,
};
use crate::ui::craft::item_label;

/// Follow the pointer and latch what it is over.
///
/// Latched on hover rather than on release, which is the reference's own
/// feel: the centre readout updates as the thumb sweeps, so the price is
/// visible *before* the commitment, and releasing is just letting go.
pub fn track(mut ui: ResMut<Ui>, window: Query<&Window, With<PrimaryWindow>>) {
    if ui.panel != Panel::Wheel {
        if ui.hover.is_some() {
            ui.hover = None;
        }
        return;
    }
    let Ok(window) = window.single() else { return };
    let Some(p) = window.cursor_position() else {
        return;
    };
    let cx = window.width() * 0.5;
    let cy = window.height() * 0.5;
    // Bevy's UI y grows downward; `pick` is written in the orientation the
    // trigonometry is readable in, so the flip happens here, once.
    let hover = crate::ui::build::pick(p.x - cx, -(p.y - cy), Rings::default());

    if hover == ui.hover {
        return;
    }
    ui.hover = hover;
    match hover {
        Some(Hover::Shape(i)) => ui.shape = i,
        Some(Hover::Material(i)) => ui.material = i,
        None => {}
    }
    ui.dirty = true;
}

/// Draw the wheel.
pub fn build_screen(commands: &mut Commands, ui: &Ui, core: &ClientCore) {
    let rings = Rings::default();
    let shape = SHAPES[ui.shape.min(SHAPES.len() - 1)];
    let material = MATERIALS[ui.material.min(MATERIALS.len() - 1)];
    let row = row_for(&core.piece_defs, shape, material);

    commands
        .spawn((
            super::PanelRoot,
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                ..default()
            },
            BackgroundColor(Color::srgba(0.02, 0.02, 0.025, 0.35)),
        ))
        .with_children(|root| {
            // The wheel box: centred by half-percent offsets and negative
            // margins, so every child can be placed in its local pixels.
            root.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Percent(50.0),
                    top: Val::Percent(50.0),
                    margin: UiRect {
                        left: Val::Px(-rings.rim),
                        top: Val::Px(-rings.rim),
                        ..default()
                    },
                    width: Val::Px(rings.rim * 2.0),
                    height: Val::Px(rings.rim * 2.0),
                    border_radius: BorderRadius::MAX,
                    ..default()
                },
                BackgroundColor(PANEL_BG),
            ))
            .with_children(|box_| {
                // The dead centre, drawn so the readout has a plate to sit
                // on and the player can see where the wheel stops picking.
                box_.spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(rings.rim - rings.dead),
                        top: Val::Px(rings.rim - rings.dead),
                        width: Val::Px(rings.dead * 2.0),
                        height: Val::Px(rings.dead * 2.0),
                        padding: UiRect::all(Val::Px(16.0)),
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::Center,
                        justify_content: JustifyContent::Center,
                        row_gap: Val::Px(4.0),
                        border: UiRect::all(Val::Px(1.0)),
                        border_radius: BorderRadius::MAX,
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.05, 0.05, 0.06, 0.95)),
                    BorderColor::all(LINE),
                ))
                .with_children(|c| readout(c, ui, core, shape, material, row));

                for (i, s) in SHAPES.iter().enumerate() {
                    let on = i == ui.shape;
                    let live = row_for(&core.piece_defs, *s, material).is_some();
                    chip(
                        box_,
                        rings,
                        (rings.split + rings.rim) * 0.5,
                        segment_angle(i, SHAPES.len()),
                        96.0,
                        30.0,
                        shape_label(*s),
                        on,
                        live,
                    );
                }
                for (i, m) in MATERIALS.iter().enumerate() {
                    let on = i == ui.material;
                    let live = row_for(&core.piece_defs, shape, *m).is_some();
                    chip(
                        box_,
                        rings,
                        (rings.dead + rings.split) * 0.5,
                        segment_angle(i, MATERIALS.len()),
                        72.0,
                        24.0,
                        material_label(*m),
                        on,
                        live,
                    );
                }
            });

            root.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    bottom: Val::Px(40.0),
                    width: Val::Percent(100.0),
                    justify_content: JustifyContent::Center,
                    ..default()
                },
                Pickable::IGNORE,
            ))
            .with_children(|b| {
                b.spawn((
                    Text::new(
                        "outer ring picks the shape   -   inner ring picks the material   \
                         -   let go of B to keep it",
                    ),
                    font(12.0),
                    TextColor(TEXT_DIM),
                ));
            });
        });
}

/// One label chip on a ring, placed along its segment's centre angle.
#[allow(clippy::too_many_arguments)]
fn chip(
    parent: &mut ChildSpawnerCommands,
    rings: Rings,
    radius: f32,
    angle: f32,
    w: f32,
    h: f32,
    text: &str,
    selected: bool,
    live: bool,
) {
    // Same convention as `pick`: 0 is up and the angle grows clockwise.
    let x = rings.rim + radius * angle.sin();
    let y = rings.rim - radius * angle.cos();
    parent
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(x - w * 0.5),
                top: Val::Px(y - h * 0.5),
                width: Val::Px(w),
                height: Val::Px(h),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(4.0)),
                ..default()
            },
            BackgroundColor(if selected { CELL_FULL } else { CELL_BG }),
            BorderColor::all(if selected { LINE_HOT } else { LINE }),
            // The wheel is picked by arithmetic, not by nodes — a chip that
            // ate the pointer would make the segment behind it dead.
            Pickable::IGNORE,
        ))
        .with_children(|c| {
            c.spawn((
                Text::new(text.to_string()),
                font_bold(12.0),
                // A segment the content has no piece for is drawn dead
                // rather than live-and-wrong.
                TextColor(if !live {
                    TEXT_SHORT
                } else if selected {
                    TEXT
                } else {
                    TEXT_DIM
                }),
                Pickable::IGNORE,
            ));
        });
}

/// The centre: what is chosen, what it is for, what it costs.
fn readout(
    parent: &mut ChildSpawnerCommands,
    _ui: &Ui,
    core: &ClientCore,
    shape: u8,
    material: u8,
    row: Option<u16>,
) {
    parent.spawn((
        Text::new(shape_label(shape).to_string()),
        font_bold(20.0),
        TextColor(TEXT),
        Pickable::IGNORE,
    ));
    parent.spawn((
        Node {
            max_width: Val::Px(150.0),
            ..default()
        },
        Text::new(shape_blurb(shape).to_string()),
        font(11.0),
        TextColor(TEXT_DIM),
        Pickable::IGNORE,
    ));

    let Some(row) = row else {
        parent.spawn((
            Text::new(
                format!("no {} {}", material_label(material), shape_label(shape)).to_lowercase(),
            ),
            font(11.0),
            TextColor(TEXT_SHORT),
            Pickable::IGNORE,
        ));
        return;
    };

    let hp = core
        .piece_defs
        .pieces
        .get(row as usize)
        .map(|p| p.hp)
        .unwrap_or(0);
    parent.spawn((
        Text::new(format!("{}  -  {hp} hp", material_label(material))),
        font_bold(12.0),
        TextColor(BADGE),
        Pickable::IGNORE,
    ));

    let (lines, n) = costs(&core.piece_defs, row, &core.inv);
    for line in lines.iter().take(n) {
        parent.spawn((
            Text::new(format!(
                "{} {} ({})",
                line.units,
                item_label(&core.catalog, line.item),
                line.have
            )),
            font_bold(11.0),
            TextColor(if line.short() { TEXT_SHORT } else { TEXT_DIM }),
            Pickable::IGNORE,
        ));
    }
}
