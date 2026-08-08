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

use super::{font, font_bold, ring, Panel, Ui, BADGE, TEXT_DIM, TEXT_SHORT};
use crate::render::icons::Icons;
use crate::ui::build::{
    costs, material_label, row_for, segment_angle, shape_blurb, shape_label, Rings, PLACE_MATERIAL,
    SHAPES,
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
    if let Some(i) = hover {
        ui.shape = i;
    }
    ui.dirty = true;
}

/// Draw the wheel.
///
/// **The shape is the reference's, since 2026-08-07.** It was a flat dark
/// disc with rounded label boxes floating on it — the operator's word was
/// *"worse than even avg programmer art"*, and the diagnosis is structural
/// rather than a colour: `building.jpeg` is a **cream annulus cut into
/// wedges** with a line-art glyph in each and the world showing through the
/// middle, and no arrangement of rectangles is that. The annulus is now a
/// baked texture ([`super::ring`]) and the labels are icons.
pub fn build_screen(commands: &mut Commands, ui: &Ui, core: &ClientCore, icons: &Icons) {
    let rings = Rings::default();
    let shape = SHAPES[ui.shape.min(SHAPES.len() - 1)];
    let material = PLACE_MATERIAL;
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
            // Barely there. The reference dims the world a little and blurs
            // it; we have no blur, and a heavy scrim in its place would hide
            // the base you are aiming the piece at.
            BackgroundColor(Color::srgba(0.02, 0.02, 0.025, 0.30)),
        ))
        .with_children(|root| {
            // The wheel box: centred by half-percent offsets and negative
            // margins, so every child can be placed in its local pixels.
            // **No background** — the middle is the world, as it is in the
            // reference, and the dark plate that used to sit here is what
            // made the whole thing read as a dialog box.
            root.spawn(Node {
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
                ..default()
            })
            .with_children(|box_| {
                // The annulus, then the two chosen wedges over it. Three
                // draws for a shape Bevy UI cannot express in nodes.
                plate(box_, rings, ring::BASE);
                plate(
                    box_,
                    rings,
                    ring::SHAPE_HI[ui.shape.min(SHAPES.len() - 1)].clone(),
                );

                // The readout sits in the dead centre, over the world — on a
                // soft disc rather than bare.
                //
                // **This is not the old dialog plate coming back.** That was
                // opaque and covered the whole wheel, which is what made it
                // read as a box; this is the dead zone only, at 0.45, and it
                // exists because the reference *blurs* the world behind its
                // centre and we have no blur. Measured the hard way: the
                // first cut had the sea horizon running straight through the
                // word "Foundation".
                box_.spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(rings.rim - rings.dead),
                        top: Val::Px(rings.rim - rings.dead),
                        width: Val::Px(rings.dead * 2.0),
                        height: Val::Px(rings.dead * 2.0),
                        padding: UiRect::all(Val::Px(10.0)),
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::Center,
                        justify_content: JustifyContent::Center,
                        row_gap: Val::Px(2.0),
                        border_radius: BorderRadius::MAX,
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.04, 0.036, 0.032, 0.72)),
                    Pickable::IGNORE,
                ))
                .with_children(|c| readout(c, ui, core, shape, material, row, icons));

                for (i, s) in SHAPES.iter().enumerate() {
                    let on = i == ui.shape;
                    let live = row_for(&core.piece_defs, *s, material).is_some();
                    glyph(
                        box_,
                        rings,
                        (rings.dead + rings.rim) * 0.5,
                        segment_angle(i, SHAPES.len()),
                        38.0,
                        icons.shape(shape_icon(*s)),
                        shape_label(*s),
                        on,
                        live,
                    );
                }
            });

            // **Clear of the hotbar.** At 40 px this ran straight through the
            // cells behind it — the hotbar sits at `bottom: 18` and is 46 px
            // tall plus its 2 px border, so anything under 68 collides. 76
            // leaves a hair of gap. Found by looking at a frame, which is the
            // only thing that finds this class of defect.
            root.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    bottom: Val::Px(76.0),
                    width: Val::Percent(100.0),
                    justify_content: JustifyContent::Center,
                    ..default()
                },
                Pickable::IGNORE,
            ))
            .with_children(|b| {
                b.spawn((
                    Text::new(
                        "the ring picks the shape   -   let go to keep it   \
                         -   left click places it",
                    ),
                    font(12.0),
                    TextColor(TEXT_DIM),
                ));
            });
        });
}

/// One full-size ring texture, laid over the wheel box.
fn plate(parent: &mut ChildSpawnerCommands, rings: Rings, image: Handle<Image>) {
    parent.spawn((
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(0.0),
            top: Val::Px(0.0),
            width: Val::Px(rings.rim * 2.0),
            height: Val::Px(rings.rim * 2.0),
            ..default()
        },
        ImageNode::new(image),
        // The wheel is picked by arithmetic, never by nodes.
        Pickable::IGNORE,
    ));
}

/// The icon file for a shape and for a material. Separate from
/// `shape_label` because a label is prose and a stem is an asset name — and
/// because `icons::STEMS` has to be greppable for the gate.
fn shape_icon(shape: u8) -> &'static str {
    match shape {
        sim_core::build::SHAPE_FOUNDATION => "shape_foundation",
        sim_core::build::SHAPE_WALL => "shape_wall",
        sim_core::build::SHAPE_DOORWAY => "shape_doorway",
        sim_core::build::SHAPE_FLOOR => "shape_floor",
        sim_core::build::SHAPE_STAIRS => "shape_stairs",
        _ => "shape_roof",
    }
}

/// One glyph on a ring, placed along its segment's centre angle.
///
/// The icon is tinted rather than swapped: white-on-transparent PNGs mean a
/// chosen wedge (dark glyph on red), an ordinary wedge (red glyph on cream)
/// and a dead one are three colours of one texture. Falls back to the label
/// when no icon was baked — an empty wedge is the dark-panel defect.
#[allow(clippy::too_many_arguments)]
fn glyph(
    parent: &mut ChildSpawnerCommands,
    rings: Rings,
    radius: f32,
    angle: f32,
    size: f32,
    icon: Option<Handle<Image>>,
    text: &str,
    selected: bool,
    live: bool,
) {
    // Same convention as `pick`: 0 is up and the angle grows clockwise.
    let x = rings.rim + radius * angle.sin();
    let y = rings.rim - radius * angle.cos();
    // On the chosen wedge the ground is red, so the glyph goes near-white;
    // everywhere else the ground is cream and the glyph is the same red.
    let tint = if !live {
        Color::srgba(0.55, 0.50, 0.47, 0.55)
    } else if selected {
        Color::srgb(0.99, 0.97, 0.95)
    } else {
        Color::srgb(0.82, 0.25, 0.16)
    };

    let mut node = parent.spawn((
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(x - size * 0.5),
            top: Val::Px(y - size * 0.5),
            width: Val::Px(size),
            height: Val::Px(size),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            ..default()
        },
        Pickable::IGNORE,
    ));
    match icon {
        Some(image) => {
            node.insert((
                ImageNode {
                    image,
                    color: tint,
                    ..default()
                },
                Pickable::IGNORE,
            ));
        }
        None => {
            node.with_children(|c| {
                c.spawn((
                    Text::new(text.to_string()),
                    font_bold(11.0),
                    TextColor(tint),
                    Pickable::IGNORE,
                ));
            });
        }
    }
}

/// The centre: what is chosen, what it is for, what it costs.
///
/// Over the world rather than on a plate, which is the reference's
/// arrangement — so everything here is near-white and the chosen shape's own
/// icon sits above it in red, the way `building.jpeg` puts a large red glyph
/// over the name.
#[allow(clippy::too_many_arguments)]
fn readout(
    parent: &mut ChildSpawnerCommands,
    _ui: &Ui,
    core: &ClientCore,
    shape: u8,
    material: u8,
    row: Option<u16>,
    icons: &Icons,
) {
    if let Some(image) = icons.shape(shape_icon(shape)) {
        parent.spawn((
            Node {
                width: Val::Px(34.0),
                height: Val::Px(34.0),
                ..default()
            },
            ImageNode {
                image,
                color: Color::srgb(0.82, 0.25, 0.16),
                ..default()
            },
            Pickable::IGNORE,
        ));
    }
    parent.spawn((
        Text::new(shape_label(shape).to_string()),
        font_bold(18.0),
        TextColor(Color::srgb(0.99, 0.98, 0.96)),
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
