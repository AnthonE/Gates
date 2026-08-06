//! The HUD and the viewmodel — `ART.md` §6 and §8's last bullet.
//!
//! "A frame with no viewmodel and no HUD reads as a flythrough, which the
//! blind reader has named on every capture so far." It is the cheapest scored
//! criterion in the art rubric and the one the browser client had first, so
//! the native client is not shipping captures without it.
//!
//! The reference set's shape, measured off `Rust Images/`: a **bottom-centre
//! hotbar** with item cells, a **right-side vitals stack** with numbers —
//! small, unobtrusive, never centred — and a **held item** in the lower
//! right of frame.
//!
//! Every number drawn here is the server's. `hp_max` or `max_food` at zero
//! means no such message has ever arrived — a shard whose content disarms
//! combat or has no `[survival]` section never sends one — and the rule that
//! `core.rs` states for those fields is honoured: draw nothing rather than
//! draw an empty bar for a player who cannot be hurt.

use bevy::prelude::*;
use sim_core::limits::HOTBAR_SLOTS;

use super::rig::EyeCam;
use super::Net;

/// A hotbar cell, by index.
#[derive(Component)]
pub struct Cell(usize);

/// The vitals readout.
#[derive(Component)]
pub struct Vitals;

/// What the build wheel last chose. Drawn beside the hotbar because a
/// selection nothing shows is a selection the player cannot trust — the
/// wheel latches a piece and does not place one yet (`render/ui/wheel.rs`),
/// so this line is the whole of its visible effect.
#[derive(Component)]
pub struct Plan;

pub fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    cam: Query<Entity, With<EyeCam>>,
) {
    // The hotbar: six cells, bottom centre.
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                bottom: Val::Px(18.0),
                width: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                column_gap: Val::Px(6.0),
                ..default()
            },
            Pickable::IGNORE,
        ))
        .with_children(|row| {
            for i in 0..HOTBAR_SLOTS {
                row.spawn((
                    Cell(i),
                    Node {
                        width: Val::Px(46.0),
                        height: Val::Px(46.0),
                        border: UiRect::all(Val::Px(2.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.05, 0.05, 0.06, 0.55)),
                    BorderColor::all(Color::srgba(0.75, 0.72, 0.62, 0.35)),
                ));
            }
        });

    // The vitals stack: right side, small, never centred.
    commands.spawn((
        Vitals,
        Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(20.0),
            right: Val::Px(22.0),
            ..default()
        },
        Text::new(""),
        TextFont {
            font_size: 17.0,
            ..default()
        },
        TextColor(Color::srgba(0.93, 0.91, 0.86, 0.92)),
    ));

    // The build plan, bottom left. Only ever text: the piece it names is a
    // client-side latch, not sim state.
    commands.spawn((
        Plan,
        Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(20.0),
            left: Val::Px(22.0),
            ..default()
        },
        Text::new(""),
        TextFont {
            font_size: 14.0,
            ..default()
        },
        TextColor(Color::srgba(0.86, 0.83, 0.76, 0.80)),
        Pickable::IGNORE,
    ));

    // The viewmodel: a held item, lower right, parented to the camera so it
    // rides the view. Not an animation and not a weapon yet — the point of
    // §8's bullet is that the frame contains evidence a person is playing.
    if let Ok(cam) = cam.single() {
        let handle = materials.add(StandardMaterial {
            base_color: Color::srgb(0.30, 0.22, 0.14),
            perceptual_roughness: 0.85,
            ..default()
        });
        let head = materials.add(StandardMaterial {
            base_color: Color::srgb(0.42, 0.43, 0.45),
            perceptual_roughness: 0.42,
            metallic: 0.65,
            ..default()
        });
        commands.entity(cam).with_children(|c| {
            // Held low and to the right, angled across the frame. The first
            // cut put it near centre at arm's length and it read as a prop
            // floating in the world rather than as something carried — the
            // reference frames all show the item entering from the lower
            // right corner and leaving frame at the bottom.
            let hold = Transform::from_xyz(0.34, -0.30, -0.46).with_rotation(Quat::from_euler(
                EulerRot::YXZ,
                -0.42,
                0.42,
                0.10,
            ));
            c.spawn((
                Mesh3d(meshes.add(Cuboid::new(0.038, 0.038, 0.52))),
                MeshMaterial3d(handle),
                hold,
            ));
            c.spawn((
                Mesh3d(meshes.add(Cuboid::new(0.16, 0.045, 0.075))),
                MeshMaterial3d(head),
                hold * Transform::from_xyz(0.0, 0.02, -0.26),
            ));
        });
    }
}

/// Redraw from the core. Cheap enough per frame: six background colours and
/// one string.
#[allow(clippy::type_complexity)]
pub fn update(
    net: NonSend<Net>,
    mut cells: Query<(&Cell, &mut BorderColor, &mut BackgroundColor)>,
    mut vitals: Query<&mut Text, (With<Vitals>, Without<Plan>)>,
    mut plan: Query<&mut Text, (With<Plan>, Without<Vitals>)>,
    // `Option`, because a capture run does not register the menus at all.
    ui: Option<Res<super::ui::Ui>>,
) {
    let core = &net.session.core;

    if let (Ok(mut text), Some(ui)) = (plan.single_mut(), ui.as_ref()) {
        use crate::ui::build::{material_label, row_for, shape_label, MATERIALS, SHAPES};
        let shape = SHAPES[ui.shape.min(SHAPES.len() - 1)];
        let material = MATERIALS[ui.material.min(MATERIALS.len() - 1)];
        // Named only when the content actually has that piece — the wheel
        // draws a dead segment for a pair the shard did not bake, and the
        // HUD must not contradict it.
        let out = match row_for(&core.piece_defs, shape, material) {
            Some(_) => format!(
                "BUILD  {} {}   (hold B)",
                material_label(material),
                shape_label(shape)
            ),
            None => "BUILD  -   (hold B)".to_string(),
        };
        if text.0 != out {
            text.0 = out;
        }
    }

    for (cell, mut border, mut bg) in cells.iter_mut() {
        let selected = cell.0 == net.sel as usize;
        *border = BorderColor::all(if selected {
            Color::srgba(0.98, 0.86, 0.55, 0.95)
        } else {
            Color::srgba(0.75, 0.72, 0.62, 0.35)
        });
        *bg = BackgroundColor(if selected {
            Color::srgba(0.14, 0.13, 0.10, 0.72)
        } else {
            Color::srgba(0.05, 0.05, 0.06, 0.55)
        });
    }

    let Ok(mut text) = vitals.single_mut() else {
        return;
    };
    let mut out = String::new();
    // Zero max means the message has never arrived, which is not the same as
    // a zeroed vital — see the header.
    if core.hp_max > 0 {
        out.push_str(&format!("HP  {}/{}\n", core.hp, core.hp_max));
    }
    if core.max_food > 0 {
        out.push_str(&format!("FOOD {}\n", core.food));
    }
    if core.max_water > 0 {
        out.push_str(&format!("WATER {}", core.water));
    }
    if text.0 != out {
        text.0 = out;
    }
}
