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
//! `browser_smoke` measured time-to-world — that gate is deleted, and the
//! reasoning is kept because it is about the join path, not about the gate.

use bevy::asset::RenderAssetUsages;
use bevy::image::Image;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use crate::ui::map::{self, MarkKind, GRID_COLS, GRID_LETTERS};

use super::menu::Screen;
use super::{ui, Net, WorldId};

/// Map texture resolution. 512 over a 2048 m island is 4 m a pixel — finer
/// than the 8 m terrain cell, so the coastline is limited by the heightfield
/// and not by the image.
pub const MAP_PX: usize = 512;

/// How large the map is drawn, in screen pixels.
const MAP_DRAW_PX: f32 = 640.0;

/// An anchor marker's edge (bed, hearth, backpack, waystation), screen px.
/// Smaller than the player square's 9 on purpose: the panel's first job is
/// *where am I*, and a marker the same size as the answer competes with it.
/// `DECISIONS.md` §open, map markers v1 — carried from the retired browser
/// row's rationale.
const MAP_MARK_PX: f32 = 7.0;

/// The haven ring's edge. Larger than an anchor — it is the island's one
/// authored destination — but drawn as an OUTLINE, so it stays visually
/// lighter than the player's solid square whatever its extent.
const HAVEN_PX: f32 = MAP_MARK_PX + 4.0;

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

impl Island {
    /// The painted island for this seed, painting it if this is the first
    /// ask of the session.
    ///
    /// **Two screens share it and that is the reason it is a method.** The
    /// death screen draws the same island (`render::death`), and a second
    /// `paint` call there would be ~65 k height taps repeated on the one
    /// frame a player is already unhappy about — plus a second texture in
    /// VRAM for the same picture. The seed check is what makes it safe to
    /// share: a second shard repaints rather than showing the first one's
    /// coastline, which looks plausible and is the worst kind of wrong.
    pub fn texture(&mut self, images: &mut Assets<Image>, seed: u64) -> Handle<Image> {
        if self.seed != Some(seed) || self.texture.is_none() {
            let mut buf = vec![0u8; MAP_PX * MAP_PX * 4];
            map::paint(seed, MAP_PX, &mut buf);
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
            self.texture = Some(images.add(image));
            self.seed = Some(seed);
        }
        self.texture.clone().expect("painted above")
    }
}

/// **Hold `G`.** The map is up while the key is down and gone when it is
/// released; `Esc` also closes it, because every other screen in this client
/// answers `Esc` and one that did not would read as a hang.
///
/// It was a `M` toggle until 2026-08-16 (`DECISIONS.md`, the control scheme),
/// and a hold is a different thing rather than the same thing with a shorter
/// press: you keep running while you read it, which is why the two systems
/// below are careful about state that a toggle could afford to be sloppy with.
///
/// **The guard is not decoration.** `open` runs `in_state(InWorld)` — which is
/// exactly where the inventory panel and the door keypad also live, since
/// neither is a `Screen` — so without it, `G` typed into the crafting search
/// box or into a keypad's code would also throw the map up. That was a live
/// bug on the old binding (`M` into the search box opened the map) and it is
/// fixed here rather than inherited: `verbs::keys` and `ghost::level_keys`
/// already stand down for the same reason, and this system simply never had
/// the check. The chat composer needs no arm — `chat::keys` clears the whole
/// keyboard while it is open.
pub fn open(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut next: ResMut<NextState<Screen>>,
    ui: Option<Res<super::panels::Ui>>,
    pad: Option<Res<super::verbs::Pad>>,
) {
    let busy = ui.map(|u| u.panel.grabs_pointer()).unwrap_or(false)
        || pad.map(|p| p.0.is_open()).unwrap_or(false);
    if !busy && keyboard.just_pressed(KeyCode::KeyG) {
        next.set(Screen::Map);
    }
}

/// Closed by letting go — `!pressed` rather than `just_released`.
///
/// The difference is a real frame and not a style choice: a tap shorter than
/// one frame produces a `just_pressed` that `open` sees and a `just_released`
/// that this system never observes, and the map would be stuck up with no key
/// held. Asking whether the key is down now cannot miss that edge.
pub fn keys(keyboard: Res<ButtonInput<KeyCode>>, mut next: ResMut<NextState<Screen>>) {
    if !keyboard.pressed(KeyCode::KeyG) || keyboard.just_pressed(KeyCode::Escape) {
        next.set(Screen::InWorld);
    }
}

/// **The pointer stays locked, and the world keeps running under it.**
///
/// This was a screen you stopped at and read, with the cursor released. A
/// held map is the opposite: the player is moving, the mouse is still
/// steering, and handing the pointer back for the duration would swing the
/// view on release when the OS cursor snapped home. `input::gather` is
/// registered to keep running in `Screen::Map` for the same reason — see
/// `render/mod.rs`, where that is also what keeps the sim's input latch fresh
/// instead of leaving the body walking on the last frame's keys.
///
/// Kept as an empty system rather than deleted so the `OnEnter`/`OnExit`
/// wiring stays visible at the registration site: this screen deliberately
/// does nothing to the cursor, which is worth reading as a decision rather
/// than as an omission.
pub fn enter() {}

pub fn leave() {}

pub fn setup(
    mut commands: Commands,
    mut island: ResMut<Island>,
    mut images: ResMut<Assets<Image>>,
    world: Res<WorldId>,
    net: NonSend<Net>,
    look: Res<super::input::Look>,
) {
    // Paint on the first open of this seed, and never again.
    let texture = island.texture(&mut images, world.seed);

    let [x, _, z] = net.session.core.predict.render_position();
    let (px, py) = map::world_to_map(x, z, 1);
    let square = map::grid_label(x, z);
    let bearing = bearing_text(look.yaw);

    // The markers: the authored destinations off the memoized `Haven`, the
    // beds and hearths off the deploy mirror, the standing death bags.
    // Resolved on OPEN, like the paint — the marked things move on the
    // timescale of raids, not frames, so a deploy placed while the screen is
    // open waits for the next M. The player marker is the one thing `track`
    // moves.
    let core = &net.session.core;
    let mut marks = map::Marks::default();
    map::resolve_marks(
        &mut marks,
        &world.haven,
        core.deploys.entries(),
        &core.deploy_defs,
        core.deploy_defs_have,
        core.bags.entries(),
        core.own_bag,
    );

    commands
        .spawn((MapRoot, ui::screen(Color::srgba(0.02, 0.02, 0.025, 0.94))))
        .with_children(|root| {
            root.spawn((
                ui::strong("MAP", 30.0, ui::TITLE),
                Node {
                    margin: UiRect::bottom(Val::Px(6.0)),
                    ..default()
                },
            ));
            root.spawn((
                ui::strong(
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
                // The marks, UNDER the player: a sibling spawned later draws
                // on top, and *where am I* outranks everything else drawn
                // here. Spawned in REVERSE resolve order, so the push order
                // is one rule with two ends — first pushed is last the cap
                // eats AND last spawned, drawing on top: the haven over a
                // bag over a stranger's bed. (Forward order quietly flipped
                // when bags moved ahead of the anchors: the beds, spawned
                // later, papered over the bag marks the rank had just
                // protected.) Positions are `resolve_marks`'s fractions —
                // the same `world_to_map` the player goes through, so the
                // two cannot disagree about the projection.
                for m in marks.a[..marks.count].iter().rev() {
                    spawn_mark(frame, m);
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

            // The cap is drop-newest and NOT silent: a truncated map that
            // says nothing reads as "everything is drawn" (`CLAUDE.md` §4's
            // stated-overflow-policy law, applied to a picture).
            if marks.dropped > 0 {
                root.spawn((
                    ui::label(
                        format!("{} marks beyond the cap", marks.dropped),
                        12.0,
                        ui::FAINT,
                    ),
                    Node {
                        margin: UiRect::top(Val::Px(6.0)),
                        ..default()
                    },
                ));
            }

            root.spawn((
                ui::label(
                    format!(
                        "{}    -    north is up    -    let go of G to close",
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

/// One mark. Shape and weight are per KIND — colour alone is the channel a
/// player reading a small panel in a hurry confuses, so no two marked kinds
/// share both:
///
/// - haven / waystation: a RING (outline only), large tier and small — a
///   place you go, not a thing you own;
/// - bed: a solid square, hearth: the same square turned 45° into a diamond,
///   backpack: a solid disc — the browser layer's shapes, kept.
///
/// The exhaustive `match` is the gate the browser needed a table-walk for: a
/// kind added without a draw branch fails to compile.
///
/// Shared with the death screen (`render::death`), which draws the same
/// markers on the same island — one projection, one shape table, so a bag
/// cannot be in two places depending on which screen you are looking at.
pub(super) fn spawn_mark(frame: &mut ChildSpawnerCommands, m: &map::Mark) {
    let f = m.kind.fill();
    let fill = Color::srgb(f[0] / 255.0, f[1] / 255.0, f[2] / 255.0);
    let px = match m.kind {
        MarkKind::Haven => HAVEN_PX,
        _ => MAP_MARK_PX,
    };
    let node = |border: f32| Node {
        position_type: PositionType::Absolute,
        left: Val::Percent(m.px * 100.0),
        top: Val::Percent(m.py * 100.0),
        width: Val::Px(px),
        height: Val::Px(px),
        margin: UiRect::axes(Val::Px(-px * 0.5), Val::Px(-px * 0.5)),
        border: UiRect::all(Val::Px(border)),
        ..default()
    };
    match m.kind {
        // Never in a live `Marks` — `count` bounds the caller's iteration —
        // and drawing nothing keeps that true on screen too.
        MarkKind::None => {}
        MarkKind::Haven | MarkKind::Waystation => {
            let mut n = node(2.0);
            n.border_radius = BorderRadius::MAX;
            frame.spawn((n, BorderColor::all(fill)));
        }
        MarkKind::Bed | MarkKind::Hearth => {
            let mut e = frame.spawn((
                node(1.0),
                BackgroundColor(fill),
                BorderColor::all(Color::srgba(0.0, 0.0, 0.0, 0.55)),
            ));
            if m.kind == MarkKind::Hearth {
                e.insert(UiTransform::from_rotation(Rot2::degrees(45.0)));
            }
        }
        // A bag inside its cooldown: the bed's square, HOLLOW. Weight is
        // the channel (`MarkKind::fill`'s note says why it is not
        // colour) — an outline reads as "this one is not answering" next
        // to a solid one at a glance, and the two are the same blue so
        // both still read as yours.
        MarkKind::BedSpent => {
            frame.spawn((node(2.0), BorderColor::all(fill)));
        }
        MarkKind::Backpack => {
            let mut n = node(1.0);
            n.border_radius = BorderRadius::MAX;
            frame.spawn((
                n,
                BackgroundColor(fill),
                BorderColor::all(Color::srgba(0.0, 0.0, 0.0, 0.55)),
            ));
        }
    }
}

/// Keep the marker under the player while the screen is open — the world is
/// still pumping behind it, so a player being chased can watch themselves
/// move.
pub fn track(net: NonSend<Net>, mut markers: Query<&mut Node, With<Marker>>) {
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

/// The heading, from the same function the compass strip reads — the map and
/// the HUD cannot disagree about which way the player is looking.
fn bearing_text(yaw: f32) -> String {
    format!("facing {:03.0}°", crate::look::bearing_deg(yaw))
}
