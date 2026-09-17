//! The map screen — `Screen::Map`, opened with `M`.
//!
//! All arithmetic is [`crate::ui::map`]'s: the palette, the hillshade and the
//! two positional facts that keep the island right side up. This file turns
//! that buffer into a texture and puts a marker on it.
//!
//! **Painted once per session, on the LOADING screen.** The island is a pure
//! function of the seed and the seed does not change inside a session, so the
//! paint is done once — but "once" and "free" are different claims and this
//! header made the second one for a year on a number nobody had measured. It
//! said "~65 k height taps ... work done once, on the first open, off the join
//! path"; the real figure at [`MAP_PX`] is `size² + 4size` taps after the
//! rolling-window fix and five times that before it, and the wall clock in
//! release is **263 ms**. On a screen you open by HOLDING `G` while the world
//! runs, that is an eight-frame freeze at the moment a player reached for the
//! map — usually because something is chasing them.
//!
//! So it is painted by [`prepaint`] while the loading bar fills, which is
//! where this client already puts work a player cannot see. `web/src/map.js`
//! deferred it off boot to keep it out of `browser_smoke`'s time-to-world
//! measurement; that gate is deleted and the reasoning survives it — the join
//! path is still the wrong place. The loading screen is not the join path.

use bevy::asset::RenderAssetUsages;
use bevy::image::Image;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use crate::ui::map::{self, MarkKind, GRID_COLS};

use super::screen::Screen;
use super::{ui, Net, WorldId};

/// Map texture resolution. 512 over a 2048 m island is 4 m a pixel — finer
/// than the 8 m terrain cell, so the coastline is limited by the heightfield
/// and not by the image.
pub const MAP_PX: usize = 512;

/// How large the map is drawn, in screen pixels.
const MAP_DRAW_PX: f32 = 640.0;

/// An anchor marker's badge (bed, hearth, backpack, waystation), screen px.
///
/// **It grew from 7 to 18 when the markers got pictures**, and the rule it
/// used to state is not repealed so much as re-aimed. That rule was "smaller
/// than the player square's 9, because the panel's first job is *where am I*
/// and a marker the same size as the answer competes with it" — correct, and
/// it was being enforced with the wrong instrument. A 7 px coloured square
/// does not lose to the player marker; it loses to the hillshade, and a
/// player reading the panel in a hurry could not tell an orange square from a
/// straw one anyway. The picture is what separates the kinds now, and a
/// picture at 7 px is four pixels of mush.
///
/// The ranking survives in the shape rather than the size: the player is the
/// only SOLID, SATURATED mark on the screen and the only one that points
/// somewhere (`PLAYER_PX`). `DECISIONS.md` §open, map markers v2.
const MAP_MARK_PX: f32 = 18.0;

/// How much of a badge the picture inside it takes. The rest is the ring and
/// the dark disc that makes a white silhouette readable over a lit island —
/// an icon drawn edge to edge has nothing separating it from the terrain.
const MARK_ICON_FRAC: f32 = 0.62;

/// The haven's badge. Larger than an anchor — it is the island's one authored
/// destination, and the two tiers are separated by size alone
/// (`ui::map::MarkKind::icon`).
const HAVEN_PX: f32 = MAP_MARK_PX + 8.0;

/// The player's arrow, screen px. The largest mark on the screen, because
/// *where am I* is the panel's first job and it is the one question that has
/// exactly one answer.
const PLAYER_PX: f32 = 22.0;

/// The site name under an authored destination's badge, screen px, and the
/// width of the row it is centred in.
///
/// The row is fixed-width so both tiers' names share one axis: a text node
/// sized to its own string hangs off the right of the badge, and `HAVEN` and
/// `WAYSTATION` would then be centred on two different things.
const SITE_LABEL_PX: f32 = 10.0;
const SITE_LABEL_W: f32 = 84.0;

/// The disc behind a solid badge's picture, and behind a hollow one's.
///
/// A white silhouette over a hillshaded island is legible over litter and
/// gone over sand — the problem `ui::TEXT_SHADOW` exists for, in the version
/// that works for a shape rather than a glyph. The hollow badge's is fainter
/// because a ring already reads as an outline and filling it solid would say
/// "solid", which is the channel a spent bed is using to say the opposite.
const BADGE_SOLID: Color = Color::srgba(0.05, 0.05, 0.06, 0.72);
const BADGE_HOLLOW: Color = Color::srgba(0.05, 0.05, 0.06, 0.42);

/// The player's ink. The only saturated, solid mark on the screen.
const PLAYER_INK: Color = Color::srgb(0.98, 0.30, 0.24);

/// The grid labels' ink. Warm off-white at low alpha — an index rather than a
/// feature, and 256 of them, so anything louder becomes the picture.
const GRID_INK: Color = Color::srgba(0.95, 0.94, 0.90, 0.45);

/// The grid label in each cell, screen px.
///
/// **Every cell, the way the reference game does it** (Devblog 181: each grid
/// is lettered along the horizontal axis and numbered along the vertical) —
/// 256 of them at [`GRID_COLS`]². It is what makes a map something two people
/// can talk over: "the hearth is in N12" is only useful if N12 is written on
/// the map rather than counted along its edge.
///
/// Small, and top-left of its own cell rather than centred in it, for the
/// reason the grid LINES are faint: the label is an index, not a feature, and
/// a number in the middle of a square sits on top of the terrain it is
/// describing. The reference's own devblog calls its first version "a little
/// bit ugly"; the corner is where a printed chart puts it.
const GRID_LABEL_PX: f32 = 9.0;

/// Everything this screen owns.
#[derive(Component)]
pub struct MapRoot;

/// The player's marker.
#[derive(Component)]
pub struct Marker;

/// The line above the island: which square you are in, and which way you are
/// facing. Its own component because [`track`] rewrites it every frame — see
/// there for why it cannot be written once at `setup`.
#[derive(Component)]
pub struct Readout;

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
    pub fn texture(
        &mut self,
        images: &mut Assets<Image>,
        seed: u64,
        haven: &sim_core::terrain::Haven,
    ) -> Handle<Image> {
        if self.seed != Some(seed) || self.texture.is_none() {
            let mut buf = vec![0u8; MAP_PX * MAP_PX * 4];
            map::paint(seed, haven, MAP_PX, &mut buf);
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

/// Paint the island while the LOADING screen is up, so the first `G` is free.
///
/// **Measured, and it is the reason this system exists at all.** `paint` is
/// ~263 ms in release at [`MAP_PX`] (it was 525 before the height field
/// stopped being sampled five times a pixel — `ui::map::paint`'s own note).
/// The map is opened by HOLDING `G` while the world runs, so painting it
/// lazily on the first open is a ~8-frame freeze at the exact moment a player
/// is usually running from something. `render/map.rs`'s header called that
/// work "done once, on the first open, off the join path" and was right about
/// the join path and wrong that off it meant free.
///
/// The loading screen is where the client already absorbs work a player
/// cannot see — the ring builders, and `prewarm` specialising every material
/// — and a quarter second there costs a quarter second of a bar that is
/// already moving. Same trade `render/prewarm.rs` makes and for the same
/// reason.
///
/// Idempotent by construction: [`Island::texture`] is memoized on the seed,
/// so every frame after the first is a handle clone. It is a `run_if` system
/// rather than an `OnEnter` one because `WorldId` is inserted when the
/// welcome lands and this state can be entered in the same frame — an
/// `OnEnter` that missed it would paint nothing and say nothing.
pub fn prepaint(
    world: Option<Res<WorldId>>,
    mut island: ResMut<Island>,
    mut images: ResMut<Assets<Image>>,
) {
    if let Some(world) = world {
        island.texture(&mut images, world.seed, &world.haven);
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
/// fixed here rather than inherited: `verbs::keys` and `ghost::height_keys`
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
    icons: Option<Res<super::icons::Icons>>,
) {
    // Already painted by `prepaint` while the loading bar filled; this is the
    // handle clone. It still PAINTS if it has to — a shard joined without ever
    // passing through `Screen::Loading` is a state this screen should survive,
    // and a map that refused to draw would be a worse answer than a stutter.
    let texture = island.texture(&mut images, world.seed, &world.haven);
    let icons = icons.as_deref();

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
        core.own_bags(),
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
                Readout,
                ui::strong(readout_text(&square, &bearing), 15.0, ui::DIM),
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
                // The grid's LINES. The labels follow, one per cell.
                //
                // ⚠ **This comment used to say the letters and numbers were
                // "drawn on the rails outside".** They were not drawn at
                // all — the only thing outside the frame was a footer
                // printing `GRID_LETTERS[..1]`, i.e. the literal string
                // `"A"`, which read as a legend and said nothing. Sixteen
                // squares a side with no way to name one is a grid you can
                // see and cannot use, and the operator asked for the
                // reference's version of it (2026-09-16).
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
                // …and the label in every one of the 256 cells, off
                // `ui::map::grid_cell_label` — the SAME function the readout
                // above the island calls, so the square a player reads off
                // the picture and the square the panel tells them they are
                // standing in cannot disagree. A second copy of `letter` +
                // `row + 1` here is the hand-kept mirror `CLAUDE.md` records
                // going stale twice.
                //
                // They carry `ui::TEXT_SHADOW` for the reason the floating
                // HUD strings do: this text has no chrome behind it and the
                // island does, so the same grey is comfortable over forest
                // and gone over beach.
                let cell = 100.0 / GRID_COLS as f32;
                for row in 0..GRID_COLS {
                    for col in 0..GRID_COLS {
                        frame.spawn((
                            ui::label(map::grid_cell_label(col, row), GRID_LABEL_PX, GRID_INK),
                            ui::TEXT_SHADOW,
                            Node {
                                position_type: PositionType::Absolute,
                                left: Val::Percent(col as f32 * cell),
                                top: Val::Percent(row as f32 * cell),
                                padding: UiRect::axes(Val::Px(3.0), Val::Px(1.0)),
                                ..default()
                            },
                        ));
                    }
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
                    spawn_mark(frame, m, icons);
                }

                // The player. `world_to_map` with size 1 gives a fraction, so
                // it places by percentage and the frame can be any size.
                //
                // **An arrow, and it points where you are looking.** It was a
                // red square with a white border, which answers *where am I*
                // and not *which way am I facing* — and the second question is
                // the one a player actually has on the map screen, because the
                // map is how you decide where to go. The bearing was already
                // on screen as three digits above the island (`facing 086°`);
                // a number is a thing you convert and an arrow is a thing you
                // read. `map_player` is drawn pointing north at rest, so the
                // rotation is the compass's own yaw with no offset — see
                // `ci/icons/map_player.svg`, which says why that was worth
                // authoring rather than taking from the archive.
                //
                // The red survives the change: it is the only saturated,
                // solid mark on the screen and nothing else may be (see
                // `MAP_MARK_PX`).
                //
                // **The node is built once, before the branch.** `insert`ing
                // a second `Node` over the first works — it replaces rather
                // than duplicating — but a bundle assembled out of two
                // helpers is the shape `CLAUDE.md` records dying at spawn
                // with "has duplicate components", inside a command queue,
                // naming no system. Not a risk worth carrying on a screen no
                // headless test can visit.
                let arrow = icons.and_then(|i| i.glyph("map_player"));
                // With no icon baked, or before the asset server settles: the
                // small square this screen shipped for a year, so the panel
                // still answers its first question. `render/icons.rs`'s rule
                // — a miss is not a defect and must not draw an empty cell.
                let size = if arrow.is_some() { PLAYER_PX } else { 9.0 };
                let mut player = frame.spawn((
                    Marker,
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Percent(px * 100.0),
                        top: Val::Percent(py * 100.0),
                        width: Val::Px(size),
                        height: Val::Px(size),
                        margin: UiRect::axes(Val::Px(-size * 0.5), Val::Px(-size * 0.5)),
                        border: UiRect::all(Val::Px(if arrow.is_some() { 0.0 } else { 2.0 })),
                        ..default()
                    },
                    UiTransform::from_rotation(Rot2::degrees(crate::look::bearing_deg(look.yaw))),
                ));
                match arrow {
                    Some(h) => {
                        player.insert(ImageNode::new(h).with_color(PLAYER_INK));
                    }
                    None => {
                        player.insert((
                            BackgroundColor(PLAYER_INK),
                            BorderColor::all(Color::srgba(1.0, 1.0, 1.0, 0.9)),
                        ));
                    }
                }
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

            // The legend. It used to open with `GRID_LETTERS[..1]` — the
            // literal string `"A"`, printed where a legend goes, saying
            // nothing about anything. The grid explains itself now, in all
            // 256 cells, so what is left is the two facts the picture cannot
            // state: which way is up, and how to make it go away.
            root.spawn((
                ui::label(
                    format!(
                        "A1 is north-west    -    {} m a square    -    north is up    \
                         -    let go of G to close",
                        map::GRID_M as u32
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

/// One mark: a badge with a PICTURE in it, and a name for authored destinations.
///
/// **The picture is the channel that carries this screen now**, and the two
/// it replaces are re-ranked rather than deleted. What was here was colour
/// plus shape — a blue square, an orange diamond, a straw disc, a white ring
/// — and both are abstractions a player has to have been taught: an orange
/// diamond means hearth only once somebody says so. A fireplace means
/// fireplace. That is why the reference game's map is readable at a glance
/// and ours was a field of coloured confetti (the operator's own frame,
/// 2026-09-16).
///
/// So:
///
/// - **the picture** says WHAT (`ui::map::MarkKind::icon`);
/// - **the colour** says whose and what state, unchanged
///   (`MarkKind::fill` — the two beds share one blue on purpose);
/// - **the shape** keeps the one distinction it was carrying that the
///   picture does not: a RING for a place you go, a solid badge for a thing
///   you own. The hearth's 45° diamond is gone, and its going is the point —
///   it was separating hearth from bed, which the fireplace and the sleeping
///   bag now do far better, and a rotated badge would rotate the picture
///   inside it.
/// - **the weight** still says whether a bed of yours will answer:
///   [`MarkKind::BedSpent`] draws the badge hollow, same blue, same picture.
///
/// A missing icon falls back to the bare badge rather than to an empty
/// square — `render/icons.rs`'s rule, and here it means a marker with no art
/// yet is still a marker in the right place.
///
/// Shared with the death screen (`render::death`), which draws the same
/// markers on the same island — one projection, one shape table, so a bag
/// cannot be in two places depending on which screen you are looking at.
/// `pub` rather than `pub(super)` since 2026-09-16, for `tests/map_marks.rs`
/// — a spawn is not type-checked for duplicate components and the only way to
/// find out is to run it, which an integration test cannot do through a
/// private function. `render::prewarm` is `pub` for the same reason.
pub fn spawn_mark(
    frame: &mut ChildSpawnerCommands,
    m: &map::Mark,
    icons: Option<&super::icons::Icons>,
) {
    let f = m.kind.fill();
    let fill = Color::srgb(f[0] / 255.0, f[1] / 255.0, f[2] / 255.0);
    let px = match m.kind {
        MarkKind::Haven => HAVEN_PX,
        _ => MAP_MARK_PX,
    };
    // A place you go is an outline; a thing you own is solid. The hollow
    // spent bed borrows the outline treatment for its own reason (weight),
    // which is why this is a `match` and not `is_authored`.
    let hollow = matches!(
        m.kind,
        MarkKind::Haven | MarkKind::Waystation | MarkKind::Depot | MarkKind::BedSpent
    );
    let mut node = Node {
        position_type: PositionType::Absolute,
        left: Val::Percent(m.px * 100.0),
        top: Val::Percent(m.py * 100.0),
        width: Val::Px(px),
        height: Val::Px(px),
        margin: UiRect::axes(Val::Px(-px * 0.5), Val::Px(-px * 0.5)),
        border: UiRect::all(Val::Px(if hollow { 2.0 } else { 1.5 })),
        align_items: AlignItems::Center,
        justify_content: JustifyContent::Center,
        ..default()
    };
    // Round for the authored tier, near-square for what is yours: the shape
    // channel, reduced to the one distinction the pictures do not make.
    node.border_radius = if hollow && m.kind != MarkKind::BedSpent {
        BorderRadius::MAX
    } else {
        BorderRadius::all(Val::Px(3.0))
    };

    if m.kind == MarkKind::None {
        // Never in a live `Marks` — `count` bounds the caller's iteration —
        // and drawing nothing keeps that true on screen too.
        return;
    }

    let mut e = frame.spawn((
        node,
        // The disc behind the picture. A white silhouette over a hillshaded
        // island is legible over litter and gone over sand, which is the
        // same problem `ui::TEXT_SHADOW` exists for; a dark plate is the
        // version of the fix that works for a shape rather than a glyph.
        BackgroundColor(if hollow { BADGE_HOLLOW } else { BADGE_SOLID }),
        BorderColor::all(fill),
    ));
    if let Some(h) = m.kind.icon().and_then(|k| icons.and_then(|i| i.glyph(k))) {
        let inner = px * MARK_ICON_FRAC;
        e.with_children(|badge| {
            badge.spawn((
                ImageNode::new(h).with_color(fill),
                Node {
                    width: Val::Px(inner),
                    height: Val::Px(inner),
                    ..default()
                },
            ));
        });
    }
    // The name, for an authored destination and nothing else — the reference
    // game's rule (Devblog 149, "Monuments are labeled on the map"), and
    // `MarkKind::site_label` carries the argument for it.
    //
    // A child of the badge, so it follows the mark by construction rather
    // than by a second copy of the projection. `left: 50%` on a fixed-width
    // centred row is what puts it under the badge's own axis: a text node
    // sized to its own string would hang off the right of the marker and the
    // two tiers' labels would not line up with each other.
    if let Some(name) = m.kind.site_label() {
        e.with_children(|badge| {
            badge
                .spawn(Node {
                    position_type: PositionType::Absolute,
                    top: Val::Percent(100.0),
                    left: Val::Percent(50.0),
                    width: Val::Px(SITE_LABEL_W),
                    margin: UiRect::left(Val::Px(-SITE_LABEL_W * 0.5)),
                    justify_content: JustifyContent::Center,
                    ..default()
                })
                .with_children(|row| {
                    row.spawn((ui::strong(name, SITE_LABEL_PX, fill), ui::TEXT_SHADOW));
                });
        });
    }
}

/// The line above the island. One function so `setup` and [`track`] cannot
/// write it two ways.
fn readout_text(square: &str, bearing: &str) -> String {
    if square.is_empty() {
        format!("off the island    -    {bearing}")
    } else {
        format!("{square}    -    {bearing}")
    }
}

/// Keep the marker under the player while the screen is open — the world is
/// still pumping behind it, so a player being chased can watch themselves
/// move.
///
/// **Three things move, not one, and the other two were stale until the
/// marker became an arrow.** The map is HELD (`keys`), so a player reads it
/// while running and turning; everything `setup` computed once — the square,
/// the bearing, and now the arrow's heading — goes out of date the moment
/// they do. A position that tracked while a bearing printed three frozen
/// digits was quietly wrong and looked fine, because nothing on screen
/// contradicted it. An arrow that does not turn while the body does is the
/// same bug made visible, which is the only reason this was found.
pub fn track(
    net: NonSend<Net>,
    look: Res<super::input::Look>,
    mut markers: Query<(&mut Node, &mut UiTransform), With<Marker>>,
    mut readout: Query<&mut Text, With<Readout>>,
) {
    let [x, _, z] = net.session.core.predict.render_position();
    let (px, py) = map::world_to_map(x, z, 1);
    if let Ok((mut node, mut xf)) = markers.single_mut() {
        node.left = Val::Percent(px * 100.0);
        node.top = Val::Percent(py * 100.0);
        // The same yaw the compass strip reads, through the same function —
        // `map_player` is drawn pointing north at rest, so this is the
        // bearing with no offset (`ci/icons/map_player.svg`).
        xf.rotation = Rot2::degrees(crate::look::bearing_deg(look.yaw));
    }
    if let Ok(mut text) = readout.single_mut() {
        let line = readout_text(&map::grid_label(x, z), &bearing_text(look.yaw));
        // Compared before writing, for `loading::update`'s reason: a `Text`
        // assigned an equal string is still a change as far as the layout is
        // concerned, and this one would re-shape a line every frame the map
        // is up.
        if text.0 != line {
            text.0 = line;
        }
    }
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
