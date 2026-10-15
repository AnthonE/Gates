//! The map screen — `Screen::Map`, held open with `G`. Fullscreen, and the
//! wheel zooms it (`ui::map::MapView`).
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
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::input::mouse::{
    AccumulatedMouseMotion, AccumulatedMouseScroll, MouseButtonInput, MouseScrollUnit,
};
use bevy::input::ButtonState;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::window::PrimaryWindow;

use crate::ui::map::{self, MarkKind, GRID_COLS};

use super::screen::Screen;
use super::{ui, Net, WorldId};

/// Map texture resolution. 512 over a 2048 m island is 4 m a pixel — finer
/// than the 8 m terrain cell, so the coastline is limited by the heightfield
/// and not by the image.
pub const MAP_PX: usize = 512;

/// How close to the window's edge the crosshair may go, UI px. Pushed past
/// it, the island slides instead (edge push).
const EDGE_PX: f32 = 12.0;

/// How far a held left button travels before it is a drag (the island
/// follows the mouse) rather than a click (a mark's colour), UI px.
const DRAG_PX: f32 = 4.0;

/// The sea around the island, and the letterbox either side of it at full
/// zoom-out: the paint's own deep water, so the edge of the picture is not a
/// line on screen.
fn sea() -> Color {
    let [r, g, b] = map::SEA_DEEP;
    Color::srgb(r / 255.0, g / 255.0, b / 255.0)
}

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
/// THE GATE is the island's hub, so it draws biggest.
const TOWN_PX: f32 = MAP_MARK_PX + 14.0;

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
const GRID_LABEL_PX: f32 = 11.0;

/// Everything this screen owns.
#[derive(Component)]
pub struct MapRoot;

/// The player's marker.
#[derive(Component)]
pub struct Marker;

/// A teammate's dot (wire v108): slot `i` of the team view, placed by
/// [`mates`] every frame — Rust shows the team on the map wherever they are.
#[derive(Component)]
pub struct MateDot(pub usize);
/// The name under a teammate's dot.
#[derive(Component)]
pub struct MateName(pub usize);

/// The line above the island: which square you are in, and which way you are
/// facing. Its own component because [`track`] rewrites it every frame — see
/// there for why it cannot be written once at `setup`.
#[derive(Component)]
pub struct Readout;

/// The marks a player put on their map (`ui::map::Pins`), kept across opens.
#[derive(Resource, Default)]
pub struct MapPins(pub map::Pins);

/// Where the map's crosshair sits: on SCREEN (UI px), where the mouse moves
/// it while the map is held instead of turning the view, and the island
/// fraction under it, which a right click marks or clears.
#[derive(Resource, Default)]
pub struct MapCursor {
    /// Screen position, UI px.
    pub sx: f32,
    pub sy: f32,
    /// The island fraction under it, kept in step by [`aim`] and [`view`].
    pub fx: f32,
    pub fy: f32,
    /// A click's reach on a mark, as an island fraction at this zoom.
    pub pick: f32,
    /// The window the map fills, UI px, as last laid out.
    pub w: f32,
    pub h: f32,
    /// A left button held down, and how far the mouse has gone since.
    drag: Option<f32>,
    /// The marks layer needs redrawing.
    dirty: bool,
    /// The mark whose label is being typed, and how it looked before, for
    /// `Esc` to put back. While it is `Some` the map stays up with `G` let
    /// go and the keyboard is the label's (`label`).
    typing: Option<(usize, map::PinStyle)>,
}

/// The help line under the island; [`aim`] rewrites it while a label is
/// being typed.
#[derive(Component)]
pub struct MapHelp;

/// The layer the player's marks are drawn in, inside the island's frame.
#[derive(Component)]
pub struct PinLayer;

/// The crosshair the mouse moves over the island.
#[derive(Component)]
pub struct MapCrosshair;

/// The island's picture and everything pinned to it. Everything inside is
/// placed by percentage, so [`view`] moving and sizing this one node is the
/// whole of the zoom.
#[derive(Component)]
pub struct IslandFrame;

/// The zoom, bottom right.
#[derive(Component)]
pub struct ZoomReadout;

/// How far in, and where: kept across opens, so the map comes back at the
/// zoom it was left at (re-centred on the player).
#[derive(Resource, Default)]
pub struct MapCamera(pub map::MapView);

/// A mark's badge, screen px (a diamond, unlike every authored mark), and
/// the picture inside it. Its colour is the player's (`map::PIN_COLOURS`).
const PIN_PX: f32 = 16.0;
const PIN_ICON_PX: f32 = 11.0;
/// The ink of a mark's number and picture, and of its edge: dark, on any of
/// its light colours.
const PIN_TEXT: Color = Color::srgb(0.12, 0.08, 0.02);
const PIN_EDGE: Color = Color::srgba(0.1, 0.07, 0.02, 0.9);

/// The painted island, kept across opens.
#[derive(Resource, Default)]
pub struct Island {
    pub texture: Option<Handle<Image>>,
    /// Which seed it was painted for, so a second shard repaints rather than
    /// showing the first one's island. Without this, disconnecting and
    /// joining elsewhere shows the wrong coastline — and it looks plausible,
    /// which is the worst kind of wrong.
    seed: Option<u64>,
    /// The blight overlay's per-texel terrain (`ui::map::BlightCache`), built
    /// while the loading bar fills: the seed-only half of the overlay.
    cache: Option<map::BlightCache>,
    /// The spread over the island, repainted in place when its moment moves.
    blight: Option<Handle<Image>>,
    /// The moment `blight` was painted at.
    blight_key: Option<BlightKey>,
}

/// What the overlay was painted for: this wipe's outbreaks, the fel tier and
/// the arc's whole hour. An hour, because the front walks ~3 m of a 16 m
/// texel in one and an outbreak a few; a tier climb repaints at once.
#[derive(Clone, Copy, PartialEq)]
struct BlightKey {
    plan: sim_core::blight::Plan,
    tier: u8,
    hour: u64,
}

/// The blight overlay inside the island's frame ([`spawn_blight_layer`]).
#[derive(Component)]
pub struct MapBlight;

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

    /// The overlay's terrain cache for this seed: ~65 k height taps, so
    /// built by [`prepaint`] and only ever reused after.
    pub fn blight_cache(&mut self, seed: u64, haven: &sim_core::terrain::Haven) {
        if self.cache.as_ref().map(|c| c.seed()) != Some(seed) {
            self.cache = Some(map::BlightCache::new(seed, haven, map::BLIGHT_PX));
            self.blight = None;
            self.blight_key = None;
        }
    }

    /// The blight overlay at this moment, repainting it in place when the
    /// plan, the tier or the arc's hour has moved since it was last painted.
    /// The handle stays the same across repaints, so nothing that shows it is
    /// touched.
    pub fn blight(
        &mut self,
        images: &mut Assets<Image>,
        world: &WorldId,
        plan: &sim_core::blight::Plan,
        tier: u8,
        tick: u64,
    ) -> Handle<Image> {
        use sim_core::blight;
        self.blight_cache(world.seed, &world.haven);
        let hour = tick / blight::TICKS_PER_HOUR;
        let key = BlightKey {
            plan: *plan,
            tier,
            hour,
        };
        let cache = self.cache.as_ref().expect("built above");
        let now = blight::now(plan, &world.haven, hour * blight::TICKS_PER_HOUR, tier);
        match self.blight.clone() {
            Some(h) => {
                if self.blight_key != Some(key) {
                    if let Some(data) = images.get_mut(&h).and_then(|img| img.data.as_mut()) {
                        map::paint_blight(cache, &now, data);
                    }
                    self.blight_key = Some(key);
                }
                h
            }
            None => {
                let px = map::BLIGHT_PX;
                let mut buf = vec![0u8; px * px * 4];
                map::paint_blight(cache, &now, &mut buf);
                let image = Image::new(
                    Extent3d {
                        width: px as u32,
                        height: px as u32,
                        depth_or_array_layers: 1,
                    },
                    TextureDimension::D2,
                    buf,
                    TextureFormat::Rgba8UnormSrgb,
                    // Both worlds: the CPU copy is what a repaint writes into.
                    RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
                );
                let h = images.add(image);
                self.blight = Some(h.clone());
                self.blight_key = Some(key);
                h
            }
        }
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
        // And the blight overlay's terrain, ~30 ms more: its paint waits for
        // the map, when the tier and the moment are this island's.
        island.blight_cache(world.seed, &world.haven);
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
    mouse: Res<ButtonInput<MouseButton>>,
    binds: Res<super::binds::Binds>,
    mut next: ResMut<NextState<Screen>>,
    ui: Option<Res<super::panels::Ui>>,
    pad: Option<Res<super::verbs::Pad>>,
) {
    let busy = ui.map(|u| u.panel.grabs_pointer()).unwrap_or(false)
        || pad.map(|p| p.0.is_open()).unwrap_or(false);
    if !busy && binds.just_pressed(super::binds::Action::Map, &keyboard, &mouse) {
        next.set(Screen::Map);
    }
}

/// Closed by letting go — `!pressed` rather than `just_released`.
///
/// The difference is a real frame and not a style choice: a tap shorter than
/// one frame produces a `just_pressed` that `open` sees and a `just_released`
/// that this system never observes, and the map would be stuck up with no key
/// held. Asking whether the key is down now cannot miss that edge.
///
/// **Except while a mark's label is being typed** (`label`): nobody types
/// with one finger on `G`, so `Enter` on a mark latches the map up until the
/// label is kept (`Enter`) or put back (`Esc`), and then it follows `G` again.
pub fn keys(
    keyboard: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    binds: Res<super::binds::Binds>,
    cursor: Res<MapCursor>,
    mut next: ResMut<NextState<Screen>>,
) {
    if cursor.typing.is_some() {
        return;
    }
    if !binds.pressed(super::binds::Action::Map, &keyboard, &mouse)
        || keyboard.just_pressed(KeyCode::Escape)
    {
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

// Each is a distinct input: the paint, the world, the body, the look, the
// icons, and the player's own marks with their crosshair.
#[allow(clippy::too_many_arguments)]
pub fn setup(
    mut commands: Commands,
    mut island: ResMut<Island>,
    mut images: ResMut<Assets<Image>>,
    world: Res<WorldId>,
    net: NonSend<Net>,
    look: Res<super::input::Look>,
    icons: Option<Res<super::icons::Icons>>,
    mut pins: ResMut<MapPins>,
    mut cursor: ResMut<MapCursor>,
    mut camera: ResMut<MapCamera>,
    windows: Query<&Window, With<PrimaryWindow>>,
    ui_scale: Res<UiScale>,
    blight: Option<Res<super::blight::Blight>>,
) {
    // Already painted by `prepaint` while the loading bar filled; this is the
    // handle clone. It still PAINTS if it has to — a shard joined without ever
    // passing through `Screen::Loading` is a state this screen should survive,
    // and a map that refused to draw would be a worse answer than a stutter.
    let texture = island.texture(&mut images, world.seed, &world.haven);
    // The spread over it, at the arc's own hour (`render/blight.rs`).
    let overlay = blight
        .as_deref()
        .map(|b| island.blight(&mut images, &world, &b.plan, b.tier, b.tick));
    let icons = icons.as_deref();

    let [x, _, z] = net.session.core.predict.render_position();
    let (px, py) = map::world_to_map(x, z, 1);
    let square = map::grid_label(x, z);
    // The crosshair opens on you; the marks are this island's.
    pins.0.for_seed(world.seed);
    // The window this screen will fill, in UI px. `view` reads the laid-out
    // root from the next frame on; this is the first frame's guess, and the
    // two agree unless the window moves between them.
    let (w, h) = windows
        .single()
        .map(|win| (win.width() / ui_scale.0, win.height() / ui_scale.0))
        .unwrap_or((1280.0, 720.0));
    // Back at the zoom it was left at, centred on you as far as the edge
    // allows; the crosshair opens on you.
    let cam = &mut camera.0;
    cam.zoom = cam.target;
    cam.centre_on(w, h, px, py);
    let (sx, sy) = cam.to_screen(w, h, px, py);
    let (left, top, side) = cam.rect(w, h);
    *cursor = MapCursor {
        sx,
        sy,
        fx: px,
        fy: py,
        pick: cam.pick_frac(w, h),
        w,
        h,
        drag: None,
        dirty: true,
        typing: None,
    };
    let bearing = bearing_text(look.yaw);

    // The markers: the authored destinations off the memoized `Haven`, the
    // beds and hearths off the deploy mirror, the standing death bags.
    // Resolved on OPEN, like the paint — the marked things move on the
    // timescale of raids, not frames, so a deploy placed while the screen is
    // open waits for the next M. The player marker is the one thing `track`
    // moves.
    let core = &net.session.core;
    // The arc's places: every work, named, and every inscribed stone, as
    // far as their rows have arrived.
    let mut arc = map::ArcMarks::default();
    for (_, w) in core.arc.known() {
        arc.add_work(w.def.spot, w.name());
    }
    for (_, s) in core.lore.known_inscriptions() {
        arc.add_stone(s.spot);
    }
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
        &arc,
    );

    // The whole window, sea-coloured, clipping the island when it is zoomed
    // past the edges. The island is one node inside it that `view` moves
    // and sizes; the chrome floats over it.
    commands
        .spawn((
            MapRoot,
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                overflow: Overflow::clip(),
                ..default()
            },
            BackgroundColor(sea()),
        ))
        .with_children(|root| {
            root.spawn((
                IslandFrame,
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(left),
                    top: Val::Px(top),
                    width: Val::Px(side),
                    height: Val::Px(side),
                    ..default()
                },
                ImageNode::new(texture),
            ))
            .with_children(|frame| {
                // The blight first: the first child draws underneath, so the
                // grid, its labels, the marks and the arrow stay over the green.
                if let Some(h) = overlay.clone() {
                    spawn_blight_layer(frame, h);
                }
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
                // Names that would print over a higher-ranked one are left
                // off (`clear_labels`); the badge still says what it is.
                let named = map::clear_labels(
                    &marks.a[..marks.count],
                    side,
                    MAP_MARK_PX,
                    SITE_LABEL_PX,
                    SITE_LABEL_W,
                );
                for (i, m) in marks.a[..marks.count].iter().enumerate().rev() {
                    spawn_mark_named(frame, m, icons, named[i]);
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
                // The player's own marks, over the authored ones and under
                // the arrow; drawn by `aim`, which owns when they change.
                frame.spawn((
                    PinLayer,
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(0.0),
                        top: Val::Px(0.0),
                        width: Val::Percent(100.0),
                        height: Val::Percent(100.0),
                        ..default()
                    },
                ));
                // Teammates (wire v108), under the arrow; [`mates`] places
                // and shows them.
                for i in 0..protocol::TEAM_MAX {
                    frame
                        .spawn((
                            MateDot(i),
                            Node {
                                position_type: PositionType::Absolute,
                                width: Val::Px(MATE_PX),
                                height: Val::Px(MATE_PX),
                                margin: UiRect::all(Val::Px(-MATE_PX * 0.5)),
                                border: UiRect::all(Val::Px(1.5)),
                                border_radius: BorderRadius::MAX,
                                ..default()
                            },
                            BackgroundColor(super::team::MATE_INK),
                            BorderColor::all(Color::srgba(0.05, 0.12, 0.05, 0.9)),
                            Visibility::Hidden,
                        ))
                        .with_children(|dot| {
                            dot.spawn((
                                MateName(i),
                                Node {
                                    position_type: PositionType::Absolute,
                                    top: Val::Px(MATE_PX + 2.0),
                                    left: Val::Px(-20.0),
                                    ..default()
                                },
                                Text::new(""),
                                ui::font(11.0),
                                TextColor(super::team::MATE_INK),
                                ui::TEXT_SHADOW,
                            ));
                        });
                }
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
            // The crosshair: a ring the mouse moves, over the island, in
            // SCREEN px — it stays where it is while the island zooms under
            // it, which is what keeps a zoom about the spot you are reading.
            root.spawn((
                MapCrosshair,
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(sx),
                    top: Val::Px(sy),
                    width: Val::Px(18.0),
                    height: Val::Px(18.0),
                    margin: UiRect::all(Val::Px(-9.0)),
                    border: UiRect::all(Val::Px(2.0)),
                    border_radius: BorderRadius::MAX,
                    ..default()
                },
                BorderColor::all(Color::srgba(1.0, 1.0, 1.0, 0.85)),
                Pickable::IGNORE,
            ));

            // The chrome floats over the picture, top left: what the map is
            // and where you are on it.
            root.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(18.0),
                    top: Val::Px(14.0),
                    flex_direction: FlexDirection::Column,
                    padding: UiRect::axes(Val::Px(14.0), Val::Px(10.0)),
                    row_gap: Val::Px(2.0),
                    ..default()
                },
                BackgroundColor(super::panels::SCRIM),
            ))
            .with_children(|tl| {
                tl.spawn(ui::strong("MAP", 26.0, ui::TITLE));
                tl.spawn((
                    Readout,
                    ui::strong(readout_text(&square, &bearing), 15.0, ui::DIM),
                ));
            });

            // Along the bottom: every gesture, the cap's note, the legend.
            root.spawn(Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                bottom: Val::Px(12.0),
                justify_content: JustifyContent::Center,
                ..default()
            })
            .with_children(|row| {
                row.spawn((
                    Node {
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::Center,
                        padding: UiRect::axes(Val::Px(14.0), Val::Px(6.0)),
                        row_gap: Val::Px(3.0),
                        ..default()
                    },
                    BackgroundColor(super::panels::SCRIM),
                ))
                .with_children(|bar| {
                    bar.spawn((MapHelp, ui::label(help_text(false), 12.0, ui::DIM)));
                    // The cap is drop-newest and NOT silent: a truncated map
                    // that says nothing reads as "everything is drawn"
                    // (`CLAUDE.md` §4's stated-overflow-policy law, applied
                    // to a picture).
                    if marks.dropped > 0 {
                        bar.spawn(ui::label(
                            format!("{} marks beyond the cap", marks.dropped),
                            12.0,
                            ui::FAINT,
                        ));
                    }
                    // The legend: the facts the picture cannot state.
                    bar.spawn(ui::label(
                        format!(
                            "A1 is north-west    -    {} m a square    -    north is up    \
                             -    let go of G to close",
                            map::GRID_M as u32
                        ),
                        12.0,
                        ui::FAINT,
                    ));
                });
            });

            // The zoom, bottom right.
            root.spawn((
                ZoomReadout,
                ui::strong(zoom_text(camera.0.target), 14.0, ui::DIM),
                ui::TEXT_SHADOW,
                Node {
                    position_type: PositionType::Absolute,
                    right: Val::Px(20.0),
                    bottom: Val::Px(18.0),
                    ..default()
                },
            ));
        });
}

/// The zoom readout: `1.0×` … `4.0×`.
fn zoom_text(zoom: f32) -> String {
    format!("{zoom:.1}\u{d7}")
}

/// Each frame the map is up: ease the zoom toward the wheel's ask about the
/// crosshair, and copy the camera onto the island's node and the crosshair's.
///
/// The window is read off the laid-out root rather than the OS window, so a
/// `UiScale` (the browser build sets one) is already divided out.
pub fn view(
    time: Res<Time>,
    mut camera: ResMut<MapCamera>,
    mut cursor: ResMut<MapCursor>,
    root: Query<&ComputedNode, With<MapRoot>>,
    mut frame: Query<&mut Node, (With<IslandFrame>, Without<MapCrosshair>)>,
    mut cross: Query<&mut Node, (With<MapCrosshair>, Without<IslandFrame>)>,
    mut zoom: Query<&mut Text, With<ZoomReadout>>,
) {
    if let Ok(c) = root.single() {
        let size = c.size() * c.inverse_scale_factor();
        if size.x > 1.0 && size.y > 1.0 {
            cursor.w = size.x;
            cursor.h = size.y;
        }
    }
    let (w, h) = (cursor.w, cursor.h);
    let cam = &mut camera.0;
    cam.ease(w, h, cursor.sx, cursor.sy, time.delta_secs());
    cam.clamp(w, h);
    let (fx, fy) = cam.to_frac(w, h, cursor.sx, cursor.sy);
    cursor.fx = fx;
    cursor.fy = fy;
    cursor.pick = cam.pick_frac(w, h);

    let (left, top, side) = cam.rect(w, h);
    if let Ok(mut node) = frame.single_mut() {
        let want = (Val::Px(left), Val::Px(top), Val::Px(side));
        if (node.left, node.top, node.width) != want {
            node.left = want.0;
            node.top = want.1;
            node.width = want.2;
            node.height = want.2;
        }
    }
    if let Ok(mut node) = cross.single_mut() {
        let want = (Val::Px(cursor.sx), Val::Px(cursor.sy));
        if (node.left, node.top) != want {
            node.left = want.0;
            node.top = want.1;
        }
    }
    if let Ok(mut text) = zoom.single_mut() {
        let want = zoom_text(cam.target);
        if text.0 != want {
            text.0 = want;
        }
    }
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
    spawn_mark_named(frame, m, icons, true);
}

/// [`spawn_mark`], with its name left off when `named` is false (it would
/// print over a higher-ranked one, `map::clear_labels`).
pub fn spawn_mark_named(
    frame: &mut ChildSpawnerCommands,
    m: &map::Mark,
    icons: Option<&super::icons::Icons>,
    named: bool,
) {
    let f = m.kind.fill();
    let fill = Color::srgb(f[0] / 255.0, f[1] / 255.0, f[2] / 255.0);
    let px = match m.kind {
        MarkKind::Haven => HAVEN_PX,
        MarkKind::Town | MarkKind::Monument => TOWN_PX,
        _ => MAP_MARK_PX,
    };
    // A place you go is an outline; a thing you own is solid. The hollow
    // spent bed borrows the outline treatment for its own reason (weight),
    // which is why this is a `match` and not `is_authored`.
    let hollow = matches!(
        m.kind,
        MarkKind::Haven
            | MarkKind::Town
            | MarkKind::Monument
            | MarkKind::Waystation
            | MarkKind::Depot
            | MarkKind::Landmark
            | MarkKind::Work
            | MarkKind::Stone
            | MarkKind::BedSpent
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

    // THE GATE's safe zone, to scale under its badge: the square inside
    // which the sentries keep the peace. The town turns by quarter turns,
    // so the square is always the map's own axes; percentages of a square
    // frame are the same on both axes.
    if m.kind == MarkKind::Town {
        let side = 2.0 * sim_core::town::SAFE_HALF_M / sim_core::terrain::ISLAND_SIZE * 100.0;
        frame.spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Percent(m.px * 100.0 - side * 0.5),
                top: Val::Percent(m.py * 100.0 - side * 0.5),
                width: Val::Percent(side),
                height: Val::Percent(side),
                border: UiRect::all(Val::Px(1.5)),
                ..default()
            },
            BackgroundColor(fill.with_alpha(0.14)),
            BorderColor::all(fill.with_alpha(0.85)),
        ));
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
    if let Some(name) = m.label_text().filter(|_| named) {
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

/// A teammate's dot, pixels.
const MATE_PX: f32 = 9.0;

/// The team's dots (wire v108): every teammate but you where the team view
/// last put them, grey asleep, red down.
pub fn mates(
    net: NonSend<Net>,
    mut dots: Query<(&MateDot, &mut Node, &mut Visibility, &mut BackgroundColor)>,
    mut names: Query<(&MateName, &mut Text)>,
) {
    let core = &net.session.core;
    let team = core.team();
    let shown = |i: usize| {
        team.members()
            .get(i)
            .filter(|m| m.id != core.player_id)
            .copied()
    };
    for (dot, mut node, mut vis, mut ink) in &mut dots {
        match shown(dot.0) {
            Some(m) => {
                let (px, py) = map::world_to_map(f32::from(m.x_m) + 0.5, f32::from(m.z_m) + 0.5, 1);
                node.left = Val::Percent(px * 100.0);
                node.top = Val::Percent(py * 100.0);
                ink.0 = super::team::ink(m.state);
                *vis = Visibility::Inherited;
            }
            None => *vis = Visibility::Hidden,
        }
    }
    for (name, mut text) in &mut names {
        let want = shown(name.0)
            .map(|m| crate::ui::names::of(core, m.id))
            .unwrap_or_default();
        if text.0 != want {
            text.0 = want;
        }
    }
}

/// The line under the island: every gesture the held map answers, so none
/// of them has to be learned from anywhere else. One function so `setup` and
/// [`aim`] cannot write it two ways.
fn help_text(typing: bool) -> &'static str {
    if typing {
        "type the label  ·  Enter: keep it  ·  Esc: put it back  ·  Backspace: rub out"
    } else {
        "mouse: aim  ·  wheel: zoom  ·  left drag: pan  ·  right click: mark the spot / clear a mark  ·  \
         on a mark: left click colour, shift + wheel picture, Enter label"
    }
}

/// While the map is held the mouse is the map's: it moves the crosshair
/// instead of turning the view (`input::gather` stands down), and pushing
/// the crosshair past the window's edge slides the island. The wheel zooms
/// about the crosshair; a held left button drags the island; a right click
/// marks the spot under the crosshair or clears the mark that is there. On a
/// mark, a left CLICK (not a drag) steps its colour and shift + wheel its
/// picture. Runs in `PreUpdate`, after the input is read and before anything
/// else sees it, and takes the buttons and the wheel for itself — so no click
/// on the map also swings, draws a bow, lights a torch or places a piece in
/// the world behind it, and no zoom also walks the hotbar.
///
/// The buttons are read off their MESSAGES, not `ButtonInput`: this system
/// resets the buttons every frame so the world never sees them, and a reset
/// button never reports `just_released` — a drag needs to see its release.
// Each is a distinct input: the mouse's four channels, the shift key, the
// marks and the camera, and the three things drawn from them.
#[allow(clippy::too_many_arguments)]
pub fn aim(
    mut commands: Commands,
    motion: Res<AccumulatedMouseMotion>,
    mut scroll: ResMut<AccumulatedMouseScroll>,
    mut buttons: MessageReader<MouseButtonInput>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
    keyboard: Res<ButtonInput<KeyCode>>,
    mut cursor: ResMut<MapCursor>,
    mut camera: ResMut<MapCamera>,
    mut pins: ResMut<MapPins>,
    icons: Option<Res<super::icons::Icons>>,
    sound: Option<ResMut<super::audio::Sound>>,
    mut help: Query<&mut Text, With<MapHelp>>,
    layer: Query<(Entity, Option<&Children>), With<PinLayer>>,
) {
    let (w, h) = (cursor.w, cursor.h);
    let cam = &mut camera.0;
    let d = motion.delta;
    if let Some(travel) = cursor.drag.as_mut() {
        // A held left button: the island follows the mouse and the
        // crosshair stays put — past a hair of travel, so a click with a
        // shaky hand is still a click.
        *travel += d.length();
        if *travel > DRAG_PX {
            cam.pan_px(w, h, d.x, d.y);
        }
    } else if d != Vec2::ZERO {
        // The crosshair moves; past the window's edge the island slides
        // under it instead (and stops when the island's edge arrives).
        let (nx, ny) = (cursor.sx + d.x, cursor.sy + d.y);
        let (cx, cy) = (
            nx.clamp(EDGE_PX, (w - EDGE_PX).max(EDGE_PX)),
            ny.clamp(EDGE_PX, (h - EDGE_PX).max(EDGE_PX)),
        );
        cam.pan_px(w, h, cx - nx, cy - ny);
        // …and never off the island: at full zoom-out the sea either side
        // is not map.
        let (l, t, side) = cam.rect(w, h);
        cursor.sx = cx.clamp(l, l + side);
        cursor.sy = cy.clamp(t, t + side);
    }
    let (fx, fy) = cam.to_frac(w, h, cursor.sx, cursor.sy);
    cursor.fx = fx;
    cursor.fy = fy;
    cursor.pick = cam.pick_frac(w, h);
    let pick = cursor.pick;

    // `input::gather`'s units: a mouse notch is a line; a trackpad's pixels
    // are a fraction of one, so it zooms smoothly rather than in jumps.
    let notches = match scroll.unit {
        MouseScrollUnit::Line => scroll.delta.y,
        MouseScrollUnit::Pixel => scroll.delta.y / 60.0,
    };
    scroll.delta = Vec2::ZERO;
    let shift = keyboard.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);

    // Not while a label is being typed: the mark it is going into must stay
    // where it is in the list, and clearing it mid-word would type into
    // whichever mark slid into its place.
    let free = cursor.typing.is_none();
    let mut pinned = false;
    for ev in buttons.read() {
        match (ev.button, ev.state) {
            (MouseButton::Left, ButtonState::Pressed) => cursor.drag = Some(0.0),
            (MouseButton::Left, ButtonState::Released) => {
                let click = cursor.drag.take().is_some_and(|t| t <= DRAG_PX);
                if let Some(i) = pins.0.hit_within(fx, fy, pick).filter(|_| click && free) {
                    pins.0.cycle_colour(i, 1);
                    cursor.dirty = true;
                }
            }
            (MouseButton::Right, ButtonState::Pressed) if free => {
                pins.0.toggle_within(fx, fy, pick);
                cursor.dirty = true;
                pinned = true;
            }
            _ => {}
        }
    }
    if notches != 0.0 {
        let on = pins.0.hit_within(fx, fy, pick).filter(|_| shift && free);
        match on {
            Some(i) => {
                pins.0.cycle_icon(i, -(notches.signum() as i32));
                cursor.dirty = true;
            }
            None => cam.wheel(notches),
        }
    }
    mouse.reset_all();
    if let Some(mut sound) = sound.filter(|_| pinned) {
        sound.play(crate::sound::mixer::Request::own(crate::sound::Cue::MapPin));
    }
    if !cursor.dirty {
        return;
    }
    let Ok((layer, kids)) = layer.single() else {
        return;
    };
    cursor.dirty = false;
    let typing = cursor.typing.map(|(i, _)| i);
    if let Ok(mut text) = help.single_mut() {
        let want = help_text(typing.is_some());
        if text.0 != want {
            text.0 = want.to_string();
        }
    }
    for k in kids.into_iter().flatten() {
        commands.entity(*k).despawn();
    }
    let icons = icons.as_deref();
    commands.entity(layer).with_children(|l| {
        for (i, (&at, &style)) in pins.0.marks().iter().zip(pins.0.styles()).enumerate() {
            spawn_pin(l, i + 1, at, style, icons, typing == Some(i));
        }
    });
}

/// `Enter` with the crosshair on a mark: type its label. The map latches up
/// (`keys`) so `G` can be let go, and until `Enter` keeps the label or `Esc`
/// puts the old one back, every key is the label's — the keyboard is wiped
/// each frame, before `input::gather` reads it, so typing `WASD` into a label
/// does not also walk the body (`chat::keys`' rule for its composer).
///
/// Runs in `PreUpdate` beside [`aim`], before anything in `Update` reads a
/// key. Reads the typed text off `KeyboardInput` rather than off key codes,
/// as chat does, so a layout's own letters come out as printed on the key.
pub fn label(
    mut keyboard: ResMut<ButtonInput<KeyCode>>,
    mut chars: MessageReader<KeyboardInput>,
    mut cursor: ResMut<MapCursor>,
    mut pins: ResMut<MapPins>,
    reports: Option<Res<super::report::Reports>>,
) {
    let Some((i, was)) = cursor.typing else {
        // The bug-report composer draws on every screen and reads `Enter`
        // itself; two text fields open at once is one eating the other.
        let free = !reports.is_some_and(|r| r.open);
        let enter = keyboard.any_just_pressed([KeyCode::Enter, KeyCode::NumpadEnter]);
        if free && enter {
            if let Some(i) = pins.0.hit_within(cursor.fx, cursor.fy, cursor.pick) {
                cursor.typing = Some((i, pins.0.style(i)));
                cursor.dirty = true;
                keyboard.reset_all();
            }
        }
        // Not typing: drain, so a key pressed before does not arrive in the
        // label the moment it opens — the `Enter` that opened it included.
        chars.clear();
        return;
    };
    let (mut done, mut changed) = (false, false);
    for ev in chars.read() {
        if done || !ev.state.is_pressed() {
            continue;
        }
        let Some(style) = pins.0.style_mut(i) else {
            done = true;
            continue;
        };
        match &ev.logical_key {
            Key::Enter => done = true,
            Key::Escape => {
                *style = was;
                done = true;
            }
            Key::Backspace => changed |= style.pop_label(),
            Key::Space => changed |= style.push_label(' '),
            Key::Character(s) => {
                for c in s.chars() {
                    changed |= style.push_label(c);
                }
            }
            _ => {}
        }
    }
    if done {
        cursor.typing = None;
    }
    cursor.dirty |= done || changed;
    keyboard.reset_all();
}

/// One of the player's own marks: a diamond in its colour with its picture
/// (or its number, `n`) in it, and its label under it in the same colour.
/// `typing` is the mark whose label is being typed: a white edge and a caret.
///
/// The diamond turns 45° and the label must not, so the two are siblings
/// under an upright anchor rather than parent and child. `pub` for
/// `tests/map_marks.rs`, which spawns every look — a bundle with a component
/// twice dies at spawn, not at compile (`spawn_mark`'s note).
pub fn spawn_pin(
    layer: &mut ChildSpawnerCommands,
    n: usize,
    (x, z): (f32, f32),
    style: map::PinStyle,
    icons: Option<&super::icons::Icons>,
    typing: bool,
) {
    let (px, py) = map::world_to_map(x, z, 1);
    let [r, g, b] = style.rgb();
    let fill = Color::srgb_u8(r, g, b);
    let picture = style
        .icon_stem()
        .and_then(|k| icons.and_then(|i| i.glyph(k)));
    let label = if typing {
        format!("{}_", style.label_raw())
    } else {
        style.label().to_string()
    };
    layer
        .spawn(Node {
            position_type: PositionType::Absolute,
            left: Val::Percent(px * 100.0),
            top: Val::Percent(py * 100.0),
            width: Val::Px(PIN_PX),
            height: Val::Px(PIN_PX),
            margin: UiRect::all(Val::Px(-PIN_PX * 0.5)),
            ..default()
        })
        .with_children(|pin| {
            let mut diamond = pin.spawn((
                Node {
                    width: Val::Percent(100.0),
                    height: Val::Percent(100.0),
                    border: UiRect::all(Val::Px(1.5)),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    ..default()
                },
                UiTransform::from_rotation(Rot2::degrees(45.0)),
                BackgroundColor(fill),
                BorderColor::all(if typing { Color::WHITE } else { PIN_EDGE }),
            ));
            // Turned back upright inside the diamond. A picture that has not
            // loaded (or was never baked) falls back to the number rather
            // than an empty badge — `render/icons.rs`'s rule.
            let upright = UiTransform::from_rotation(Rot2::degrees(-45.0));
            match picture {
                Some(h) => {
                    diamond.with_child((
                        ImageNode::new(h).with_color(PIN_TEXT),
                        Node {
                            width: Val::Px(PIN_ICON_PX),
                            height: Val::Px(PIN_ICON_PX),
                            ..default()
                        },
                        upright,
                    ));
                }
                None => {
                    diamond.with_child((ui::strong(format!("{n}"), 10.0, PIN_TEXT), upright));
                }
            }
            // Just under the diamond's lower point (half a diagonal below
            // its centre: 1.21 sides below the anchor's top), centred on its
            // axis the way a site's name is (`spawn_mark_named`).
            if !label.is_empty() {
                pin.spawn(Node {
                    position_type: PositionType::Absolute,
                    top: Val::Px(PIN_PX * 1.25),
                    left: Val::Percent(50.0),
                    width: Val::Px(SITE_LABEL_W),
                    margin: UiRect::left(Val::Px(-SITE_LABEL_W * 0.5)),
                    justify_content: JustifyContent::Center,
                    ..default()
                })
                .with_child((ui::strong(label, SITE_LABEL_PX, fill), ui::TEXT_SHADOW));
            }
        });
}

/// The blight overlay, one bundle filling the island's frame: it zooms and
/// pans with the frame for free. Public for the headless spawn test.
pub fn spawn_blight_layer(frame: &mut ChildSpawnerCommands, h: Handle<Image>) {
    frame.spawn((
        MapBlight,
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(0.0),
            top: Val::Px(0.0),
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            ..default()
        },
        ImageNode::new(h),
    ));
}

/// Keep the overlay current while the map is held open: an arc hour or a
/// tier turning over repaints it in place. A key compare otherwise.
pub fn blight_follow(
    world: Res<WorldId>,
    blight: Option<Res<super::blight::Blight>>,
    mut island: ResMut<Island>,
    mut images: ResMut<Assets<Image>>,
) {
    if let Some(b) = blight.as_deref() {
        island.blight(&mut images, &world, &b.plan, b.tier, b.tick);
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
