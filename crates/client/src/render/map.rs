//! The map screen — `Screen::Map`, opened with `M`.
//!
//! All arithmetic is [`crate::ui::map`]'s: the palette, the hillshade and the
//! two positional facts that keep the island right side up. This file turns
//! that buffer into a texture and puts a marker on it.
//!
//! **Painted once per session, lazily.** The island is a pure function of the
//! seed, the seed does not change inside a session, and the paint is ~65 k
//! height taps plus an apron — real work, but work done once, on the first
//! open, off the join path. `web/src/map.js` reached the same conclusion for
//! the same reason and is explicit that doing it at boot would put it where
//! `browser_smoke` measures time-to-world.

use bevy::asset::RenderAssetUsages;
use bevy::image::Image;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};

use crate::look::yaw_u16;
use crate::ui::map::{self, GRID_COLS, GRID_LETTERS};

use super::menu::Screen;
use super::{ui, Net, WorldId};

/// Map texture resolution. 512 over a 2048 m island is 4 m a pixel — finer
/// than the 8 m terrain cell, so the coastline is limited by the heightfield
/// and not by the image.
pub const MAP_PX: usize = 512;

/// How large the map is drawn, in screen pixels.
const MAP_DRAW_PX: f32 = 640.0;

/// Everything this screen owns.
#[derive(Component)]
pub struct MapRoot;

/// The player's marker.
#[derive(Component)]
pub struct Marker;

/// The painted island, kept across opens.
#[derive(Resource, Default)]
pub struct Island {
    pub texture: Option<Handle<Image>>,
    /// Which seed it was painted for, so a second shard repaints rather than
    /// showing the first one's island. Without this, disconnecting and
    /// joining elsewhere shows the wrong coastline — and it looks plausible,
    /// which is the worst kind of wrong.
    seed: Option<u64>,
}

/// `M` opens the map from the world; `M` or `Esc` closes it.
pub fn open(keyboard: Res<ButtonInput<KeyCode>>, mut next: ResMut<NextState<Screen>>) {
    if keyboard.just_pressed(KeyCode::KeyM) {
        next.set(Screen::Map);
    }
}

pub fn keys(keyboard: Res<ButtonInput<KeyCode>>, mut next: ResMut<NextState<Screen>>) {
    if keyboard.just_pressed(KeyCode::KeyM) || keyboard.just_pressed(KeyCode::Escape) {
        next.set(Screen::InWorld);
    }
}

/// Let the pointer go: this is a screen you read, not an overlay you fight
/// under.
pub fn enter(mut cursor: Query<&mut CursorOptions, With<PrimaryWindow>>) {
    if let Ok(mut c) = cursor.single_mut() {
        c.grab_mode = CursorGrabMode::None;
        c.visible = true;
    }
}

pub fn leave(mut cursor: Query<&mut CursorOptions, With<PrimaryWindow>>) {
    if let Ok(mut c) = cursor.single_mut() {
        c.grab_mode = CursorGrabMode::Locked;
        c.visible = false;
    }
}

pub fn setup(
    mut commands: Commands,
    mut island: ResMut<Island>,
    mut images: ResMut<Assets<Image>>,
    world: Res<WorldId>,
    net: NonSend<Net>,
    look: Res<super::input::Look>,
) {
    // Paint on the first open of this seed, and never again.
    if island.seed != Some(world.seed) || island.texture.is_none() {
        let mut buf = vec![0u8; MAP_PX * MAP_PX * 4];
        map::paint(world.seed, MAP_PX, &mut buf);
        let image = Image::new(
            Extent3d {
                width: MAP_PX as u32,
                height: MAP_PX as u32,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            buf,
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::RENDER_WORLD,
        );
        island.texture = Some(images.add(image));
        island.seed = Some(world.seed);
    }
    let texture = island.texture.clone().expect("painted above");

    let [x, _, z] = net.session.core.predict.render_position();
    let (px, py) = map::world_to_map(x, z, 1);
    let square = map::grid_label(x, z);
    let bearing = bearing_text(look.yaw);

    commands
        .spawn((MapRoot, ui::screen(Color::srgba(0.02, 0.02, 0.025, 0.94))))
        .with_children(|root| {
            root.spawn((
                ui::label("MAP", 30.0, ui::TITLE),
                Node {
                    margin: UiRect::bottom(Val::Px(6.0)),
                    ..default()
                },
            ));
            root.spawn((
                ui::label(
                    if square.is_empty() {
                        format!("off the island    -    {bearing}")
                    } else {
                        format!("{square}    -    {bearing}")
                    },
                    15.0,
                    ui::DIM,
                ),
                Node {
                    margin: UiRect::bottom(Val::Px(10.0)),
                    ..default()
                },
            ));

            root.spawn((
                Node {
                    width: Val::Px(MAP_DRAW_PX),
                    height: Val::Px(MAP_DRAW_PX),
                    border: UiRect::all(Val::Px(1.0)),
                    ..default()
                },
                BorderColor::all(ui::RULE),
                ImageNode::new(texture),
            ))
            .with_children(|frame| {
                // The grid. Lines only — the letters and numbers are drawn on
                // the rails outside, because a label inside the frame sits on
                // top of the terrain it is describing.
                for i in 1..GRID_COLS {
                    let t = i as f32 / GRID_COLS as f32 * 100.0;
                    frame.spawn((
                        Node {
                            position_type: PositionType::Absolute,
                            left: Val::Percent(t),
                            top: Val::Px(0.0),
                            width: Val::Px(1.0),
                            height: Val::Percent(100.0),
                            ..default()
                        },
                        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.18)),
                    ));
                    frame.spawn((
                        Node {
                            position_type: PositionType::Absolute,
                            top: Val::Percent(t),
                            left: Val::Px(0.0),
                            height: Val::Px(1.0),
                            width: Val::Percent(100.0),
                            ..default()
                        },
                        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.18)),
                    ));
                }
                // The marker. `world_to_map` with size 1 gives a fraction, so
                // it places by percentage and the frame can be any size.
                frame.spawn((
                    Marker,
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Percent(px * 100.0),
                        top: Val::Percent(py * 100.0),
                        width: Val::Px(9.0),
                        height: Val::Px(9.0),
                        margin: UiRect::axes(Val::Px(-4.5), Val::Px(-4.5)),
                        border: UiRect::all(Val::Px(2.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.98, 0.30, 0.24)),
                    BorderColor::all(Color::srgba(1.0, 1.0, 1.0, 0.9)),
                ));
            });

            root.spawn((
                ui::label(
                    format!(
                        "{}    -    north is up    -    M or Esc closes",
                        &GRID_LETTERS[..1.min(GRID_LETTERS.len())]
                    ),
                    12.0,
                    ui::FAINT,
                ),
                Node {
                    margin: UiRect::top(Val::Px(10.0)),
                    ..default()
                },
            ));
        });
}

/// Keep the marker under the player while the screen is open — the world is
/// still pumping behind it, so a player being chased can watch themselves
/// move.
pub fn track(
    net: NonSend<Net>,
    mut markers: Query<&mut Node, With<Marker>>,
) {
    let Ok(mut node) = markers.single_mut() else {
        return;
    };
    let [x, _, z] = net.session.core.predict.render_position();
    let (px, py) = map::world_to_map(x, z, 1);
    node.left = Val::Percent(px * 100.0);
    node.top = Val::Percent(py * 100.0);
}

pub fn teardown(mut commands: Commands, roots: Query<Entity, With<MapRoot>>) {
    for e in roots.iter() {
        commands.entity(e).despawn();
    }
}

/// Forget the painted island when the shard goes: the next one has its own
/// seed and its own coastline.
pub fn forget(mut island: ResMut<Island>) {
    *island = Island::default();
}

/// The heading, in the compass's own words.
fn bearing_text(yaw: f32) -> String {
    // Quantized like every other bearing this client reports, so the map and
    // the compass strip cannot disagree by a degree at the boundary.
    let (fx, fz) = sim_core::yaw_dir(yaw_u16(yaw));
    let deg = fx.atan2(fz).to_degrees().rem_euclid(360.0);
    format!("facing {deg:03.0}°")
}
